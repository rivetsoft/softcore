//! What a frame is made of: the cursor, the lock screen or else Softcore's own panel, the
//! shell's layers, and the windows with their borders and shadows.

use std::time::Duration;

use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree};
use smithay::backend::renderer::gles::element::PixelShaderElement;
use smithay::backend::renderer::gles::{GlesPixelProgram, GlesRenderer, Uniform, UniformName, UniformType};
use smithay::desktop::utils::send_frames_surface_tree;
use smithay::desktop::{PopupManager, layer_map_for_output};
use smithay::input::pointer::{CursorImageStatus, CursorImageSurfaceData};
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Physical, Point, Rectangle, Scale};
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::wlr_layer::Layer;

use crate::layout::{Rect, Tile};
use crate::state::{BorderKey, State};

smithay::backend::renderer::element::render_elements! {
    pub Elem<=GlesRenderer>;
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
    Shader=PixelShaderElement,
}

/// How far a shadow reaches past its window.
const SHADOW_REACH: i32 = 28;

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

float rounded(vec2 p, vec2 half_size, float r) {
    vec2 q = abs(p) - half_size + r;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}

void main() {
    vec2 p = v_coords * size - size * 0.5;
    float outer = rounded(p, size * 0.5, u_radius);
    float inner = rounded(p, size * 0.5 - u_width, max(u_radius - u_width, 0.0));
    float a = clamp(0.5 - outer, 0.0, 1.0) * clamp(inner + 0.5, 0.0, 1.0);
    gl_FragColor = u_color * a * alpha;
}
"#;

/// A soft shadow around a rounded box, a little lower than the box, with the box cut out so
/// nothing shows through a window that isn't opaque.
const SHADOW_SHADER: &str = r#"
precision mediump float;
uniform float alpha;
uniform vec2 size;
varying vec2 v_coords;
uniform float u_reach;
uniform float u_radius;

float rounded(vec2 p, vec2 half_size, float r) {
    vec2 q = abs(p) - half_size + r;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}

void main() {
    vec2 p = v_coords * size - size * 0.5;
    vec2 box = size * 0.5 - u_reach;
    float d = rounded(p - vec2(0.0, u_reach * 0.25), box, u_radius);
    float sigma = u_reach * 0.4;
    float a = 0.42 * exp(-max(d, 0.0) * max(d, 0.0) / (2.0 * sigma * sigma));
    float cut = clamp(rounded(p, box, u_radius) + 0.5, 0.0, 1.0);
    gl_FragColor = vec4(0.0, 0.0, 0.0, 1.0) * a * cut * alpha;
}
"#;

#[derive(Clone, Debug)]
pub struct Shaders {
    border: GlesPixelProgram,
    shadow: GlesPixelProgram,
}

impl Shaders {
    pub fn compile(renderer: &mut GlesRenderer) -> Option<Shaders> {
        let border = renderer.compile_custom_pixel_shader(BORDER_SHADER, &[UniformName::new("u_color", UniformType::_4f), UniformName::new("u_radius", UniformType::_1f), UniformName::new("u_width", UniformType::_1f)]);
        let shadow = renderer.compile_custom_pixel_shader(SHADOW_SHADER, &[UniformName::new("u_reach", UniformType::_1f), UniformName::new("u_radius", UniformType::_1f)]);
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

/// A window's border, kept between frames and touched only when something about it changes.
fn border_element(state: &mut State, t: &Tile, color: [u8; 3], radius: i32) -> Option<Elem> {
    let shaders = state.shaders.clone()?;
    let width = state.config.layout.border;
    let key = BorderKey { rect: t.rect, color, width, radius };
    let area = logical(Rect::new(t.rect.x - width, t.rect.y - width, t.rect.w + 2 * width, t.rect.h + 2 * width));
    let uniforms = || vec![Uniform::new("u_color", [color[0] as f32 / 255.0, color[1] as f32 / 255.0, color[2] as f32 / 255.0, 1.0]), Uniform::new("u_radius", radius as f32), Uniform::new("u_width", width as f32)];
    let win = state.wins.get_mut(&t.id)?;
    match &mut win.border {
        Some((e, k)) if *k == key => return Some(Elem::Shader(e.clone())),
        Some((e, k)) => {
            e.resize(area, None);
            e.update_uniforms(uniforms());
            *k = key;
        }
        None => win.border = Some((PixelShaderElement::new(shaders.border, area, None, 1.0, uniforms(), Kind::Unspecified), key)),
    }
    win.border.as_ref().map(|(e, _)| Elem::Shader(e.clone()))
}

fn shadow_element(state: &mut State, t: &Tile, radius: i32) -> Option<Elem> {
    let shaders = state.shaders.clone()?;
    let b = state.config.layout.border + SHADOW_REACH;
    let rect = Rect::new(t.rect.x - b, t.rect.y - b, t.rect.w + 2 * b, t.rect.h + 2 * b);
    let win = state.wins.get_mut(&t.id)?;
    match &mut win.shadow {
        Some((e, r)) if *r == rect => return Some(Elem::Shader(e.clone())),
        Some((e, r)) => {
            e.resize(logical(rect), None);
            *r = rect;
        }
        None => {
            let uniforms = vec![Uniform::new("u_reach", SHADOW_REACH as f32), Uniform::new("u_radius", radius as f32)];
            win.shadow = Some((PixelShaderElement::new(shaders.shadow, logical(rect), None, 1.0, uniforms, Kind::Unspecified), rect));
        }
    }
    win.shadow.as_ref().map(|(e, _)| Elem::Shader(e.clone()))
}

fn window_elements(state: &mut State, renderer: &mut GlesRenderer, t: &Tile, scale: Scale<f64>, out: &mut Vec<Elem>) {
    let Some(win) = state.wins.get(&t.id) else { return };
    let Some(surface) = win.window.toplevel().map(|t| t.wl_surface().clone()) else { return };
    let at = Point::<i32, Logical>::from((t.rect.x, t.rect.y)) - win.window.geometry().loc;
    let geo_loc = win.window.geometry().loc;
    let look = state.ui.look(&state.config);
    let radius = if state.config.layout.border > 0 { look.radius } else { 0 };
    let alpha = if t.active || t.fullscreen { 1.0 } else { state.config.inactive_opacity };
    popup_elements(renderer, &surface, at + geo_loc, scale, out);
    if t.border && !t.fullscreen {
        let color = if t.active { look.active } else { look.inactive };
        out.extend(border_element(state, t, color, radius));
    }
    surface_elements(renderer, &surface, at, scale, alpha, out);
    if state.config.shadows && !t.fullscreen {
        out.extend(shadow_element(state, t, radius));
    }
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
