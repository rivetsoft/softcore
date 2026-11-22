//! Blur behind see-through windows and shell surfaces.
//!
//! What is behind is drawn into a picture of its own, blurred there (shrunk by halves and
//! grown back, each step smearing a little: cheap, and soft at any strength), and the piece
//! behind each window is drawn under it. Two such pictures a display: the wallpaper, which
//! is all there is behind a tiled window and changes rarely, so its blur is made once and
//! kept; and everything under the shell's panels, windows included, made again only when
//! something under them changed.

use std::collections::HashMap;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::texture::TextureRenderElement;
use smithay::backend::renderer::element::{Id, Kind};
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexProgram, GlesTexture, Uniform, UniformName, UniformType};
use smithay::backend::renderer::{Bind, Color32F, Frame, Offscreen, Renderer, Texture};
use smithay::utils::{Logical, Physical, Point, Rectangle, Size, Transform};

use crate::layout::Rect;
use crate::render::Elem;

const HEAD: &str = r#"#version 100

//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

precision highp float;
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif

uniform float alpha;
varying vec2 v_coords;
uniform vec2 u_half;
uniform float u_offset;
"#;

const DOWN: &str = r#"
void main() {
    vec2 o = u_half * u_offset;
    vec4 sum = texture2D(tex, v_coords) * 4.0;
    sum += texture2D(tex, v_coords - o);
    sum += texture2D(tex, v_coords + o);
    sum += texture2D(tex, v_coords + vec2(o.x, -o.y));
    sum += texture2D(tex, v_coords - vec2(o.x, -o.y));
    gl_FragColor = vec4((sum / 8.0).rgb, 1.0) * alpha;
}
"#;

const UP: &str = r#"
void main() {
    vec2 o = u_half * u_offset;
    vec4 sum = texture2D(tex, v_coords + vec2(-o.x * 2.0, 0.0));
    sum += texture2D(tex, v_coords + vec2(-o.x, o.y)) * 2.0;
    sum += texture2D(tex, v_coords + vec2(0.0, o.y * 2.0));
    sum += texture2D(tex, v_coords + vec2(o.x, o.y)) * 2.0;
    sum += texture2D(tex, v_coords + vec2(o.x * 2.0, 0.0));
    sum += texture2D(tex, v_coords + vec2(o.x, -o.y)) * 2.0;
    sum += texture2D(tex, v_coords + vec2(0.0, -o.y * 2.0));
    sum += texture2D(tex, v_coords + vec2(-o.x, -o.y)) * 2.0;
    gl_FragColor = vec4((sum / 12.0).rgb, 1.0) * alpha;
}
"#;

#[derive(Clone, Debug)]
pub struct Programs {
    down: GlesTexProgram,
    up: GlesTexProgram,
}

impl Programs {
    pub fn compile(renderer: &mut GlesRenderer) -> Option<Programs> {
        let names = || [UniformName::new("u_half", UniformType::_2f), UniformName::new("u_offset", UniformType::_1f)];
        let down = renderer.compile_custom_texture_shader(format!("{HEAD}{DOWN}"), &names());
        let up = renderer.compile_custom_texture_shader(format!("{HEAD}{UP}"), &names());
        match (down, up) {
            (Ok(down), Ok(up)) => Some(Programs { down, up }),
            (d, u) => {
                tracing::warn!("the blur shaders didn't compile ({:?} {:?}): nothing is blurred", d.err(), u.err());
                None
            }
        }
    }
}

/// How many times a strength (1 to 10) halves the picture, and how far each step reaches.
fn steps(strength: i32) -> (usize, f32) {
    let s = strength.clamp(1, 10);
    // Each halving doubles the reach; the half steps between come from reaching a little
    // further, never so far that the smear shows its samples.
    ((1 + s / 2) as usize, 1.0 + (s % 2) as f32 * 0.5)
}

/// Who a piece of a blurred picture is drawn for: each needs a name of its own in a frame.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum For {
    Window(u32),
    Layer(u32),
}

/// What is behind, sharp and blurred, for one display.
pub struct Backdrop {
    size: Size<i32, Physical>,
    scale: f64,
    strength: i32,
    tracker: OutputDamageTracker,
    sharp: GlesTexture,
    /// Half the size, a quarter, …
    small: Vec<GlesTexture>,
    blurred: GlesTexture,
    /// A name for each piece drawn, new whenever the picture is: that is how what is drawn
    /// over it learns it has to be drawn again.
    ids: HashMap<For, Id>,
    fresh: bool,
}

fn one(renderer: &mut GlesRenderer, from: &GlesTexture, to: &mut GlesTexture, program: &GlesTexProgram, offset: f32) -> Option<()> {
    let (src, size) = (from.size(), to.size());
    let dest = Rectangle::<i32, Physical>::from_size((size.w, size.h).into());
    let mut fb = renderer.bind(to).ok()?;
    let mut frame = renderer.render(&mut fb, dest.size, Transform::Normal).ok()?;
    frame.clear(Color32F::BLACK, &[dest]).ok()?;
    let uniforms = [Uniform::new("u_half", [0.5 / src.w as f32, 0.5 / src.h as f32]), Uniform::new("u_offset", offset)];
    frame.render_texture_from_to(from, Rectangle::from_size((src.w as f64, src.h as f64).into()), dest, &[dest], &[], Transform::Normal, 1.0, Some(program), &uniforms).ok()?;
    let _ = frame.finish().ok()?;
    Some(())
}

impl Backdrop {
    fn new(renderer: &mut GlesRenderer, size: Size<i32, Physical>, scale: f64, strength: i32) -> Option<Backdrop> {
        let mut make = |by: i32| -> Option<GlesTexture> { renderer.create_buffer(Fourcc::Abgr8888, ((size.w / by).max(1), (size.h / by).max(1)).into()).ok() };
        let sharp = make(1)?;
        let blurred = make(1)?;
        let small = (1..=steps(strength).0).map(|i| make(1 << i)).collect::<Option<Vec<_>>>()?;
        Some(Backdrop { size, scale, strength, tracker: OutputDamageTracker::new(size, scale, Transform::Normal), sharp, small, blurred, ids: HashMap::new(), fresh: true })
    }

    /// Bring the picture up to date with what is behind now. Nothing is done when nothing
    /// there changed.
    fn update(&mut self, renderer: &mut GlesRenderer, programs: &Programs, elements: &[Elem], clear: [f32; 4]) -> Option<()> {
        let age = if std::mem::take(&mut self.fresh) { 0 } else { 1 };
        let changed = {
            let mut fb = renderer.bind(&mut self.sharp).ok()?;
            self.tracker.render_output(renderer, &mut fb, age, elements, clear).ok()?.damage.is_some()
        };
        if !changed {
            return Some(());
        }
        let offset = steps(self.strength).1;
        let n = self.small.len();
        one(renderer, &self.sharp, &mut self.small[0], &programs.down, offset)?;
        for i in 1..n {
            let (a, b) = self.small.split_at_mut(i);
            one(renderer, &a[i - 1], &mut b[0], &programs.down, offset)?;
        }
        for i in (1..n).rev() {
            let (a, b) = self.small.split_at_mut(i);
            one(renderer, &b[0], &mut a[i - 1], &programs.up, offset)?;
        }
        one(renderer, &self.small[0], &mut self.blurred, &programs.up, offset)?;
        self.ids.clear();
        Some(())
    }

    /// The piece of the blurred picture behind `rect` (in the display's own logical pixels).
    pub fn piece(&mut self, renderer: &GlesRenderer, who: For, rect: Rect, alpha: f32) -> Option<Elem> {
        // Only what is on the display: there is no picture of anything else.
        let whole = self.size.to_f64().to_logical(self.scale);
        let (x0, y0) = (rect.x.max(0), rect.y.max(0));
        let (x1, y1) = ((rect.x + rect.w).min(whole.w.ceil() as i32), (rect.y + rect.h).min(whole.h.ceil() as i32));
        if x1 <= x0 || y1 <= y0 || alpha <= 0.0 {
            return None;
        }
        let id = self.ids.entry(who).or_insert_with(Id::new).clone();
        let at = Point::<f64, Logical>::from((x0 as f64, y0 as f64)).to_physical(self.scale);
        // The picture has a pixel for each of the display's own.
        let src = Rectangle::<f64, Logical>::new((x0 as f64 * self.scale, y0 as f64 * self.scale).into(), ((x1 - x0) as f64 * self.scale, (y1 - y0) as f64 * self.scale).into());
        Some(Elem::Texture(TextureRenderElement::from_static_texture(id, renderer.context_id(), at, self.blurred.clone(), 1, Transform::Normal, Some(alpha), Some(src), Some((x1 - x0, y1 - y0).into()), None, Kind::Unspecified)))
    }
}

/// The blurred pictures of one display.
#[derive(Default)]
pub struct Behind {
    /// The wallpaper: what is behind every window.
    pub wall: Option<Backdrop>,
    /// Everything under the shell's panels.
    pub scene: Option<Backdrop>,
}

#[derive(Default)]
pub struct Blur {
    pub programs: Option<Programs>,
    pub outputs: HashMap<String, Behind>,
}

/// Bring one of a display's pictures up to date, making it first when there is none of the
/// right size and strength. None when it couldn't be made: then nothing is blurred.
pub fn update(slot: &mut Option<Backdrop>, renderer: &mut GlesRenderer, programs: &Programs, size: Size<i32, Physical>, scale: f64, strength: i32, elements: &[Elem], clear: [f32; 4]) {
    if slot.as_ref().is_none_or(|b| b.size != size || b.scale != scale || b.strength != strength) {
        *slot = Backdrop::new(renderer, size, scale, strength);
    }
    if let Some(b) = slot
        && b.update(renderer, programs, elements, clear).is_none()
    {
        *slot = None;
    }
}
