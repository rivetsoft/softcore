//! What a frame is made of: the cursor, the lock screen or else Softcore's own panel, the
//! shell's layers, and the windows with their borders and shadows.

use std::time::Duration;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::texture::TextureRenderElement;
use smithay::backend::renderer::element::utils::RescaleRenderElement;
use smithay::backend::renderer::element::{Id, Kind};
use smithay::backend::renderer::{Bind, Offscreen, Renderer};
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree};
use smithay::backend::renderer::gles::element::PixelShaderElement;
use smithay::backend::renderer::gles::{GlesPixelProgram, GlesRenderer, GlesTexture, Uniform, UniformName, UniformType};
use smithay::desktop::utils::send_frames_surface_tree;
use smithay::desktop::{PopupManager, layer_map_for_output};
use smithay::input::pointer::{CursorImageStatus, CursorImageSurfaceData};
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Physical, Point, Rectangle, Scale, Transform};
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::wlr_layer::Layer;

use crate::layout::{Rect, Tile, WinId};
use crate::state::State;

smithay::backend::renderer::element::render_elements! {
    pub Elem<=GlesRenderer>;
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
    Shader=PixelShaderElement,
    Scaled=RescaleRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>,
    Texture=TextureRenderElement<GlesTexture>,
}

/// A ring with round outer corners, and inner corners rounded by the same centre: drawn over
/// the window, it gives the window round corners too.
const BORDER_SHADER: &str = r#"
precision mediump float;
uniform float alpha;
uniform vec2 size;
varying vec2 v_coords;
uniform vec4 u_color;
uniform float u_radius;
uniform float u_width;
uniform float u_alpha;

float rounded(vec2 p, vec2 half_size, float r) {
    vec2 q = abs(p) - half_size + r;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}

void main() {
    vec2 p = v_coords * size - size * 0.5;
    float outer = rounded(p, size * 0.5, u_radius);
    float inner = rounded(p, size * 0.5 - u_width, max(u_radius - u_width, 0.0));
    float a = clamp(0.5 - outer, 0.0, 1.0) * clamp(inner + 0.5, 0.0, 1.0);
    gl_FragColor = u_color * a * alpha * u_alpha;
}
"#;

/// A soft shadow around a rounded box, a little lower than the box, with the box cut out so
/// nothing shows through a window that isn't opaque.
const SHADOW_SHADER: &str = r#"
precision mediump float;
uniform float alpha;
uniform vec2 size;
varying vec2 v_coords;
uniform float u_margin;
uniform float u_reach;
uniform float u_offset;
uniform float u_radius;
uniform vec4 u_color;
uniform float u_alpha;

float rounded(vec2 p, vec2 half_size, float r) {
    vec2 q = abs(p) - half_size + r;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}

void main() {
    vec2 p = v_coords * size - size * 0.5;
    vec2 box = size * 0.5 - u_margin;
    float d = rounded(p - vec2(0.0, u_offset), box, u_radius);
    float sigma = u_reach * 0.4;
    float a = u_color.a * exp(-max(d, 0.0) * max(d, 0.0) / (2.0 * sigma * sigma));
    float cut = clamp(rounded(p, box, u_radius) + 0.5, 0.0, 1.0);
    gl_FragColor = vec4(u_color.rgb, 1.0) * a * cut * alpha * u_alpha;
}
"#;

#[derive(Clone, Debug)]
pub struct Shaders {
    border: GlesPixelProgram,
    shadow: GlesPixelProgram,
}

impl Shaders {
    pub fn compile(renderer: &mut GlesRenderer) -> Option<Shaders> {
        let border = renderer.compile_custom_pixel_shader(BORDER_SHADER, &[UniformName::new("u_color", UniformType::_4f), UniformName::new("u_radius", UniformType::_1f), UniformName::new("u_width", UniformType::_1f), UniformName::new("u_alpha", UniformType::_1f)]);
        let shadow = renderer.compile_custom_pixel_shader(SHADOW_SHADER, &["u_margin", "u_reach", "u_offset", "u_radius", "u_alpha"].map(|n| UniformName::new(n, UniformType::_1f)).into_iter().chain([UniformName::new("u_color", UniformType::_4f)]).collect::<Vec<_>>());
        match (border, shadow) {
            (Ok(border), Ok(shadow)) => Some(Shaders { border, shadow }),
            (b, s) => {
                tracing::warn!("the border and shadow shaders didn't compile ({:?} {:?}): windows are drawn without them", b.err(), s.err());
                None
            }
        }
    }
}

fn logical(r: Rect) -> Rectangle<i32, Logical> {
    Rectangle::new((r.x, r.y).into(), (r.w, r.h).into())
}

pub fn surface_elements(renderer: &mut GlesRenderer, surface: &WlSurface, at: Point<i32, Logical>, scale: Scale<f64>, alpha: f32, out: &mut Vec<Elem>) {
    let loc: Point<i32, Physical> = at.to_physical_precise_round(scale);
    out.extend(render_elements_from_surface_tree::<_, Elem>(renderer, surface, loc, scale, alpha, Kind::Unspecified));
}

/// A surface's menus and tooltips, which go above everything else of its window.
fn popup_elements(renderer: &mut GlesRenderer, surface: &WlSurface, at: Point<i32, Logical>, scale: Scale<f64>, out: &mut Vec<Elem>) {
    for (popup, offset) in PopupManager::popups_for_surface(surface) {
        surface_elements(renderer, popup.wl_surface(), at + offset - popup.geometry().loc, scale, 1.0, out);
    }
}

fn layer_elements(renderer: &mut GlesRenderer, output: &Output, layer: Layer, scale: Scale<f64>, out: &mut Vec<Elem>) {
    let map = layer_map_for_output(output);
    // Menus of every surface on the layer first, so a neighbour never covers one.
    let layers: Vec<_> = map.layers_on(layer).rev().filter_map(|l| map.layer_geometry(l).map(|g| (l.wl_surface().clone(), g.loc))).collect();
    for (surface, loc) in &layers {
        popup_elements(renderer, surface, *loc, scale, out);
    }
    for (surface, loc) in &layers {
        surface_elements(renderer, surface, *loc, scale, 1.0, out);
    }
}

fn cursor_elements(state: &mut State, renderer: &mut GlesRenderer, output: &Output, scale: Scale<f64>, out: &mut Vec<Elem>) {
    let geo = State::output_geometry(output);
    if !geo.to_f64().contains(state.pointer_pos) {
        return;
    }
    let at = state.pointer_pos - geo.loc.to_f64();
    match state.cursor_status.clone() {
        CursorImageStatus::Hidden => {}
        CursorImageStatus::Surface(surface) => {
            let hotspot = with_states(&surface, |s| s.data_map.get::<CursorImageSurfaceData>().map(|d| d.lock().unwrap().hotspot).unwrap_or_default());
            surface_elements(renderer, &surface, at.to_i32_round() - hotspot, scale, 1.0, out);
        }
        CursorImageStatus::Named(icon) => {
            let Some((buffer, hot)) = state.cursors.get(icon) else { return };
            let loc = (at - Point::from((hot.0 as f64, hot.1 as f64))).to_physical(scale);
            if let Ok(e) = MemoryRenderBufferRenderElement::from_buffer(renderer, loc, &buffer, None, None, None, Kind::Cursor) {
                out.push(Elem::Memory(e));
            }
        }
    }
}

/// What a border was last drawn with.
#[derive(Clone, Copy, PartialEq, Debug)]
struct BorderKey {
    rect: Rect,
    color: [u8; 3],
    width: i32,
    radius: i32,
    alpha: f32,
}

/// What a shadow was last drawn with. `rect` is the window with its border.
#[derive(Clone, Copy, PartialEq, Debug)]
struct ShadowKey {
    rect: Rect,
    radius: i32,
    size: i32,
    offset: i32,
    opacity: f32,
    color: [u8; 3],
    alpha: f32,
}

/// A window's border and shadow, kept from frame to frame and touched only when something
/// about them changes, so a still window costs nothing to draw again.
#[derive(Default)]
pub struct Deco {
    border: Option<(PixelShaderElement, BorderKey)>,
    shadow: Option<(PixelShaderElement, ShadowKey)>,
}

fn rgb(c: [u8; 3], a: f32) -> [f32; 4] {
    [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, a]
}

impl Deco {
    fn border(&mut self, shaders: &Shaders, key: BorderKey) -> Elem {
        let w = key.width;
        let area = logical(Rect::new(key.rect.x - w, key.rect.y - w, key.rect.w + 2 * w, key.rect.h + 2 * w));
        let uniforms = vec![Uniform::new("u_color", rgb(key.color, 1.0)), Uniform::new("u_radius", key.radius as f32), Uniform::new("u_width", w as f32), Uniform::new("u_alpha", key.alpha)];
        match &mut self.border {
            Some((e, k)) => {
                if *k != key {
                    e.resize(area, None);
                    e.update_uniforms(uniforms);
                    *k = key;
                }
            }
            None => self.border = Some((PixelShaderElement::new(shaders.border.clone(), area, None, 1.0, uniforms, Kind::Unspecified), key)),
        }
        Elem::Shader(self.border.as_ref().unwrap().0.clone())
    }

    fn shadow(&mut self, shaders: &Shaders, key: ShadowKey) -> Elem {
        let m = key.size + key.offset.abs();
        let area = logical(Rect::new(key.rect.x - m, key.rect.y - m, key.rect.w + 2 * m, key.rect.h + 2 * m));
        let uniforms = vec![
            Uniform::new("u_margin", m as f32),
            Uniform::new("u_reach", key.size as f32),
            Uniform::new("u_offset", key.offset as f32),
            Uniform::new("u_radius", key.radius as f32),
            Uniform::new("u_color", rgb(key.color, key.opacity)),
            Uniform::new("u_alpha", key.alpha),
        ];
        match &mut self.shadow {
            Some((e, k)) => {
                if *k != key {
                    e.resize(area, None);
                    e.update_uniforms(uniforms);
                    *k = key;
                }
            }
            None => self.shadow = Some((PixelShaderElement::new(shaders.shadow.clone(), area, None, 1.0, uniforms, Kind::Unspecified), key)),
        }
        Elem::Shader(self.shadow.as_ref().unwrap().0.clone())
    }
}

/// How a window looks around its edges, the same for one that is there and one that is leaving.
struct Edges {
    border: bool,
    shadow: bool,
    color: [u8; 3],
    radius: i32,
    alpha: f32,
}

impl Edges {
    fn border(&self, state: &State, deco: &mut Deco, rect: Rect) -> Option<Elem> {
        let shaders = state.shaders.as_ref().filter(|_| self.border)?;
        Some(deco.border(shaders, BorderKey { rect, color: self.color, width: state.config.layout.border, radius: self.radius, alpha: self.alpha }))
    }

    fn shadow(&self, state: &State, deco: &mut Deco, rect: Rect) -> Option<Elem> {
        let c = &state.config;
        let shaders = state.shaders.as_ref().filter(|_| self.shadow && c.shadows && c.shadow_opacity > 0.0)?;
        let b = if self.border { c.layout.border } else { 0 };
        let rect = Rect::new(rect.x - b, rect.y - b, rect.w + 2 * b, rect.h + 2 * b);
        Some(deco.shadow(shaders, ShadowKey { rect, radius: self.radius, size: c.shadow_size, offset: c.shadow_offset, opacity: c.shadow_opacity, color: c.shadow_color, alpha: self.alpha }))
    }
}

/// Fast at first, gentle at the end.
fn ease(p: f64) -> f64 {
    1.0 - (1.0 - p.clamp(0.0, 1.0)).powi(3)
}

/// How much smaller a window is as it starts to appear, and as it has gone.
const SMALL: f64 = 0.92;

/// A rectangle made smaller (or larger) about its middle.
fn scaled(r: Rect, by: f64) -> Rect {
    let w = ((r.w as f64 * by).round() as i32).max(1);
    let h = ((r.h as f64 * by).round() as i32).max(1);
    Rect::new(r.x + (r.w - w) / 2, r.y + (r.h - h) / 2, w, h)
}

fn window_elements(state: &mut State, renderer: &mut GlesRenderer, t: &Tile, scale: Scale<f64>, out: &mut Vec<Elem>) {
    let Some(mut win) = state.wins.remove(&t.id) else { return };
    if let Some(surface) = win.window.toplevel().map(|t| t.wl_surface().clone()) {
        let geo = win.window.geometry();
        let look = state.ui.look(&state.config);
        // Appearing: it fades in, growing a little.
        let shown = ease(win.shown);
        let rect = if shown < 1.0 { scaled(t.rect, SMALL + (1.0 - SMALL) * shown) } else { t.rect };
        let edges = Edges {
            border: t.border && !t.fullscreen,
            shadow: !t.fullscreen,
            color: if t.active { look.active } else { look.inactive },
            radius: if state.config.layout.border > 0 { look.radius } else { 0 },
            alpha: shown as f32,
        };
        let alpha = if t.active || t.fullscreen { 1.0 } else { state.config.inactive_opacity } * shown as f32;
        // On its way somewhere, a window is drawn at the size it has got to, whatever size
        // its own picture is by now: the client resizes when it likes, the slide stays smooth.
        let stretch = if (t.moving || shown < 1.0) && geo.size.w > 0 && geo.size.h > 0 && (rect.w, rect.h) != (geo.size.w, geo.size.h) {
            let grow = if t.moving { 1.0 } else { rect.w as f64 / t.rect.w as f64 };
            Some(if t.moving { Scale::from((rect.w as f64 / geo.size.w as f64, rect.h as f64 / geo.size.h as f64)) } else { Scale::from(grow) })
        } else {
            None
        };
        let at = Point::<i32, Logical>::from((rect.x, rect.y));
        if stretch.is_none() {
            popup_elements(renderer, &surface, at, scale, out);
        }
        out.extend(edges.border(state, &mut win.deco, rect));
        match stretch {
            None => surface_elements(renderer, &surface, at - geo.loc, scale, alpha, out),
            Some(by) => {
                let origin: Point<i32, Physical> = at.to_physical_precise_round(scale);
                let loc: Point<i32, Physical> = (at - geo.loc).to_physical_precise_round(scale);
                let parts: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = render_elements_from_surface_tree(renderer, &surface, loc, scale, alpha, Kind::Unspecified);
                out.extend(parts.into_iter().map(|e| Elem::Scaled(RescaleRenderElement::from_element(e, origin, by))));
            }
        }
        out.extend(edges.shadow(state, &mut win.deco, rect));
    }
    state.wins.insert(t.id, win);
}

/// A window that has closed, fading out where it stood: a picture of its last frame.
pub struct Closing {
    output: String,
    rect: Rect,
    texture: GlesTexture,
    id: Id,
    deco: Deco,
    edges: Edges,
    /// From 0 to 1, when it is gone.
    pub t: f64,
}

/// Keep a picture of a window that is about to leave, so it can fade out.
pub fn start_closing(state: &mut State, id: WinId) {
    if !state.config.animations {
        return;
    }
    let Some(output) = state.wins.get(&id).and_then(|w| w.output.as_deref()).and_then(|n| state.output_named(n)).cloned() else { return };
    let Some(t) = state.tiles(&output).into_iter().find(|t| t.id == id) else { return };
    remember(state, id);
    let Some(texture) = state.wins.get_mut(&id).and_then(|w| w.picture.take()) else { return };
    let look = state.ui.look(&state.config);
    let edges = Edges {
        border: t.border && !t.fullscreen,
        shadow: !t.fullscreen,
        color: if t.active { look.active } else { look.inactive },
        radius: if state.config.layout.border > 0 { look.radius } else { 0 },
        alpha: 1.0,
    };
    state.closing.push(Closing { output: output.name(), rect: t.rect, texture, id: Id::new(), deco: Deco::default(), edges, t: 0.0 });
}

/// Take a window's picture while it has one to take: just before its client takes the frame
/// away, or the window goes. By the time it is gone there is nothing left to draw (which is
/// why a program that is killed outright just vanishes: its frame went with it).
pub fn remember(state: &mut State, id: WinId) {
    if !state.config.animations || state.wins.get(&id).is_none_or(|w| w.picture.is_some()) {
        return;
    }
    let scale = state.wins.get(&id).and_then(|w| w.output.as_deref()).and_then(|n| state.output_named(n)).map(|o| o.current_scale().fractional_scale()).unwrap_or(1.0);
    let Some(mut backend) = state.backend.take() else { return };
    let texture = snapshot(state, backend.renderer(), id, scale);
    state.backend = Some(backend);
    if let Some(w) = state.wins.get_mut(&id) {
        w.picture = texture;
    }
}

fn snapshot(state: &State, renderer: &mut GlesRenderer, id: WinId, scale: f64) -> Option<GlesTexture> {
    let size = state.source_size(&crate::capture::Source::Window(id))?;
    let win = state.wins.get(&id)?;
    let surface = win.window.toplevel()?.wl_surface().clone();
    let mut elements = Vec::new();
    surface_elements(renderer, &surface, Point::from((0, 0)) - win.window.geometry().loc, Scale::from(scale), 1.0, &mut elements);
    if elements.is_empty() {
        return None;
    }
    let mut texture: GlesTexture = renderer.create_buffer(Fourcc::Abgr8888, (size.w, size.h).into()).ok()?;
    {
        let mut fb = renderer.bind(&mut texture).ok()?;
        OutputDamageTracker::new(size, scale, Transform::Normal).render_output(renderer, &mut fb, 0, &elements, [0.0, 0.0, 0.0, 0.0]).ok()?;
    }
    Some(texture)
}

fn closing_elements(state: &mut State, renderer: &mut GlesRenderer, output: &Output, scale: Scale<f64>, out: &mut Vec<Elem>) {
    let mut closing = std::mem::take(&mut state.closing);
    let name = output.name();
    for c in closing.iter_mut().rev().filter(|c| c.output == name) {
        let gone = ease(c.t);
        let rect = scaled(c.rect, 1.0 - (1.0 - SMALL) * gone);
        c.edges.alpha = 1.0 - gone as f32;
        out.extend(c.edges.border(state, &mut c.deco, rect));
        let loc = Point::<i32, Logical>::from((rect.x, rect.y)).to_f64().to_physical(scale);
        out.push(Elem::Texture(TextureRenderElement::from_static_texture(c.id.clone(), renderer.context_id(), loc, c.texture.clone(), 1, Transform::Normal, Some(c.edges.alpha), None, Some((rect.w, rect.h).into()), None, Kind::Unspecified)));
        out.extend(c.edges.shadow(state, &mut c.deco, rect));
    }
    state.closing = closing;
}

/// Everything on a display, top first, and the colour behind it all.
pub fn elements(state: &mut State, renderer: &mut GlesRenderer, output: &Output, cursor: bool, panels: bool) -> (Vec<Elem>, [f32; 4]) {
    let scale = Scale::from(output.current_scale().fractional_scale());
    let mut out = Vec::new();
    if state.monitors_off {
        return (out, [0.0, 0.0, 0.0, 1.0]);
    }
    if cursor {
        cursor_elements(state, renderer, output, scale, &mut out);
    }
    if state.lock.locked {
        // Nothing of the session is drawn while locked, whether or not the lock screen has
        // shown up yet.
        if let Some((_, s)) = state.lock.surfaces.iter().find(|(o, _)| o == output) {
            surface_elements(renderer, s.wl_surface(), (0, 0).into(), scale, 1.0, &mut out);
        }
        return (out, [0.0, 0.0, 0.0, 1.0]);
    }

    let size = State::output_size(output);
    if panels
        && state.active_output() == Some(output)
        && let Some((buffer, (w, h))) = state.ui.panel_buffer(&state.config, state.mod_is_alt(), (size.w, size.h), state.picker.as_mut())
    {
        let loc = Point::<i32, Logical>::from(((size.w - w) / 2, (size.h - h) / 2)).to_f64().to_physical(scale);
        if let Ok(e) = MemoryRenderBufferRenderElement::from_buffer(renderer, loc, &buffer, None, None, None, Kind::Unspecified) {
            out.push(Elem::Memory(e));
        }
    }

    let tiles = state.tiles(output);
    layer_elements(renderer, output, Layer::Overlay, scale, &mut out);
    for t in tiles.iter().rev().filter(|t| t.fullscreen) {
        window_elements(state, renderer, t, scale, &mut out);
    }
    layer_elements(renderer, output, Layer::Top, scale, &mut out);
    closing_elements(state, renderer, output, scale, &mut out);
    for t in tiles.iter().rev().filter(|t| !t.fullscreen) {
        window_elements(state, renderer, t, scale, &mut out);
    }
    layer_elements(renderer, output, Layer::Bottom, scale, &mut out);
    layer_elements(renderer, output, Layer::Background, scale, &mut out);

    let bg = state.ui.look(&state.config).background;
    (out, [bg[0] as f32 / 255.0, bg[1] as f32 / 255.0, bg[2] as f32 / 255.0, 1.0])
}

/// After a frame is shown: tell clients they may draw the next one. Windows out of sight are
/// told once a second, so they keep working without drawing at full speed.
pub fn frame_done(state: &State, output: &Output) {
    let time = state.start.elapsed();
    let visible: Vec<_> = state.tiles(output).iter().map(|t| t.id).collect();
    let to = |_: &WlSurface, _: &smithay::wayland::compositor::SurfaceData| Some(output.clone());
    for (id, win) in &state.wins {
        if win.output.as_deref() != Some(output.name().as_str()) {
            continue;
        }
        let throttle = if visible.contains(id) { Duration::ZERO } else { Duration::from_millis(995) };
        win.window.send_frame(output, time, Some(throttle), to);
    }
    for w in &state.unmapped {
        w.send_frame(output, time, Some(Duration::ZERO), to);
    }
    for layer in layer_map_for_output(output).layers() {
        layer.send_frame(output, time, Some(Duration::ZERO), to);
    }
    for (o, s) in &state.lock.surfaces {
        if o == output {
            send_frames_surface_tree(s.wl_surface(), output, time, Some(Duration::ZERO), to);
        }
    }
    if let CursorImageStatus::Surface(s) = &state.cursor_status {
        send_frames_surface_tree(s, output, time, Some(Duration::ZERO), to);
    }
}
