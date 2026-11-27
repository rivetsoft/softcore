//! Softcore's own title bars: a bar over a window with its name and buttons to float it,
//! to fill the room with it and to close it. They are painted in the Rivet look (ui.rs), in
//! the Rivet theme's colours unless the settings name others, so they follow the wallpaper
//! as the borders do. Dragging a bar moves its window (or picks its column up), and a
//! double click fills the room with it.

use std::time::Duration;

use smithay::input::pointer::CursorImageStatus;
use smithay::utils::{Logical, Point};
use smithay::wayland::shell::wlr_layer::Layer;

use crate::layout::{Rect, WinId};
use crate::state::State;

/// What part of a title bar is under the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Bar,
    /// Floats the window, or puts it back in a column.
    Float,
    /// Fills the room with the window, or gives it its size back.
    Maximize,
    Close,
}

/// How a bar is painted: when any of it changes, it is painted again.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub title: String,
    pub size: (i32, i32),
    /// Pixels to a point on the display it is painted for.
    pub scale: i32,
    pub active: bool,
    pub hover: Option<Part>,
    pub floating: bool,
    pub maximized: bool,
    /// The buttons go on the bar's left.
    pub left: bool,
    /// Which look it was painted in, so a new wallpaper paints it again.
    pub look: u64,
}

/// What the pointer has to do with title bars.
#[derive(Default)]
pub struct Bars {
    /// The bar and part under the pointer, for the button it is over to show it.
    pub hover: Option<(WinId, Part)>,
    /// The last press on a bar, for a second one to make a double click.
    last: Option<(WinId, Duration)>,
    /// A press on a bar was kept from the windows, so its release is too.
    pub eat_release: bool,
}

/// How soon a second click counts as a double click.
const DOUBLE: Duration = Duration::from_millis(400);

/// The buttons on a bar this wide and tall, each a square as tall as the bar, from the outer
/// edge in: close, maximize, float.
pub fn buttons(w: i32, h: i32, left: bool) -> [(Part, Rect); 3] {
    let at = |i: i32| if left { i * h } else { w - (i + 1) * h };
    [(Part::Close, Rect::new(at(0), 0, h, h)), (Part::Maximize, Rect::new(at(1), 0, h, h)), (Part::Float, Rect::new(at(2), 0, h, h))]
}

fn inside(r: &Rect, x: f64, y: f64) -> bool {
    x >= r.x as f64 && x < (r.x + r.w) as f64 && y >= r.y as f64 && y < (r.y + r.h) as f64
}

/// The window whose title bar is at a point, and the part of the bar there. None where a
/// window, a panel or nothing at all is on top.
pub fn under(state: &State, pos: Point<f64, Logical>) -> Option<(WinId, Part)> {
    if state.overview.on || state.lock.locked {
        return None;
    }
    let output = state.output_at(pos)?;
    if state.layer_under(pos).is_some_and(|l| matches!(l.layer(), Layer::Top | Layer::Overlay)) {
        return None;
    }
    let at = pos - State::output_geometry(output).loc.to_f64();
    for t in state.tiles(output).iter().rev().filter(|t| !t.hidden) {
        if inside(&t.rect, at.x, at.y) {
            return None;
        }
        let bar = Rect::new(t.rect.x, t.rect.y - t.bar, t.rect.w, t.bar);
        if t.bar > 0 && inside(&bar, at.x, at.y) {
            let (x, y) = (at.x - bar.x as f64, at.y - bar.y as f64);
            let part = buttons(bar.w, bar.h, state.config.title_bar_buttons_left).into_iter().find(|(_, r)| inside(r, x, y)).map_or(Part::Bar, |(p, _)| p);
            return Some((t.id, part));
        }
    }
    None
}

/// The pointer moved, with nothing held: the button under it shows it.
pub fn hover(state: &mut State, pos: Point<f64, Logical>) {
    let now = under(state, pos);
    if now != state.bars.hover {
        if now.is_some() {
            // Off a window and onto Softcore's own bar: the window's cursor no longer applies.
            state.cursor_status = CursorImageStatus::default_named();
        }
        state.bars.hover = now;
        state.queue_redraw();
    }
}

/// Fill the room with a window, or give it back its size: a floating one fills the room
/// on its display, a column the width of the screen.
pub fn maximize(state: &mut State, id: WinId) {
    state.layout.focus_window(id);
    if state.layout.is_floating(id) {
        state.layout.toggle_float_maximized(id);
    } else {
        state.layout.toggle_maximized();
    }
}

/// A button pressed with the pointer: true when it was on a title bar, which then had it.
pub fn pressed(state: &mut State, button: u32) -> bool {
    let Some((id, part)) = under(state, state.pointer_pos) else { return false };
    state.bars.eat_release = true;
    state.layout.focus_window(id);
    state.layer_focus = None;
    state.update_focus();
    let now = state.start.elapsed();
    let left = button == 0x110;
    match part {
        _ if !left => {}
        Part::Close => {
            if let Some(top) = state.wins.get(&id).and_then(|w| w.window.toplevel()) {
                top.send_close();
            }
        }
        Part::Maximize => maximize(state, id),
        Part::Float => state.layout.toggle_floating(id),
        Part::Bar => {
            if state.bars.last.is_some_and(|(w, at)| w == id && now - at < DOUBLE) {
                state.bars.last = None;
                maximize(state, id);
            } else {
                state.bars.last = Some((id, now));
                // The release that ends the drag is kept from the window as the press was.
                state.bars.eat_release = !crate::float::start(state, id, false, None, true);
            }
        }
    }
    state.queue_redraw();
    true
}
