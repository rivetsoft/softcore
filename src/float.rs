//! Floating windows moved and resized: with the pointer (Mod and the left button moves, Mod
//! and the right button resizes from the nearest corner; a window's own title bar and edges
//! work too, when it has them), and with the keys that move and size columns.

use smithay::input::pointer::{CursorIcon, CursorImageStatus, MotionEvent};
use smithay::utils::{Logical, Point, SERIAL_COUNTER};

use crate::config::{Action, WidthChange};
use crate::layout::{Dir, Float, Rect, WinId};
use crate::state::State;

/// How far a key moves a floating window.
const STEP: i32 = 40;

pub struct Drag {
    id: WinId,
    /// Resizing, by the left edge and by the top edge (else the right, the bottom). None: moving.
    resize: Option<(bool, bool)>,
    /// Where the pointer and the window were when it began.
    from: Point<f64, Logical>,
    rect: Rect,
    /// The button press that began it was kept from the window, so its release is too.
    pub quiet: bool,
}

/// Where a floating window is on its display now, and the part of the display it has.
fn place(state: &State, id: WinId) -> Option<(Rect, Rect)> {
    if !state.layout.is_floating(id) {
        return None;
    }
    let area = state.layout.area_of(id)?;
    let output = state.wins.get(&id)?.output.as_deref().and_then(|n| state.output_named(n))?;
    let rect = state.tiles(output).into_iter().find(|t| t.id == id && !t.fullscreen)?.rect;
    Some((rect, area))
}

fn put(state: &mut State, id: WinId, rect: Rect, area: Rect, sized: bool, hold: (bool, bool)) {
    let size = if sized { Some((rect.w, rect.h)) } else { state.layout.float(id).size };
    state.layout.set_float(id, Float { pos: Some((rect.x - area.x, rect.y - area.y)), size, hold });
    state.queue_redraw();
}

/// Begin moving or resizing a floating window with the pointer. `corner`: the edges to
/// resize by; None picks the corner nearest the pointer.
pub fn start(state: &mut State, id: WinId, resize: bool, corner: Option<(bool, bool)>, quiet: bool) -> bool {
    let Some((rect, _)) = place(state, id) else { return false };
    let from = state.pointer_pos;
    let resize = resize.then(|| {
        corner.unwrap_or_else(|| {
            let origin = state.wins.get(&id).and_then(|w| w.output.as_deref()).and_then(|n| state.output_named(n)).map(|o| State::output_geometry(o).loc).unwrap_or_default();
            ((from.x - origin.x as f64) < (rect.x + rect.w / 2) as f64, (from.y - origin.y as f64) < (rect.y + rect.h / 2) as f64)
        })
    });
    state.cursor_status = CursorImageStatus::Named(match resize {
        None => CursorIcon::Grabbing,
        Some((true, true)) => CursorIcon::NwResize,
        Some((false, true)) => CursorIcon::NeResize,
        Some((true, false)) => CursorIcon::SwResize,
        Some((false, false)) => CursorIcon::SeResize,
    });
    state.drag = Some(Drag { id, resize, from, rect, quiet });
    state.queue_redraw();
    true
}

/// The pointer moved while a window is held.
pub fn moved(state: &mut State) {
    let Some(d) = &state.drag else { return };
    let (id, r, resize) = (d.id, d.rect, d.resize);
    let (dx, dy) = ((state.pointer_pos.x - d.from.x).round() as i32, (state.pointer_pos.y - d.from.y).round() as i32);
    let Some(area) = state.layout.area_of(id).filter(|_| state.layout.is_floating(id)) else {
        end(state);
        return;
    };
    match resize {
        None => put(state, id, Rect::new(r.x + dx, r.y + dy, r.w, r.h), area, false, (false, false)),
        Some((left, top)) => {
            let w = (if left { r.w - dx } else { r.w + dx }).max(80);
            let h = (if top { r.h - dy } else { r.h + dy }).max(60);
            // The corner across from the one held stays where it is.
            let x = if left { r.x + r.w - w } else { r.x };
            let y = if top { r.y + r.h - h } else { r.y };
            put(state, id, Rect::new(x, y, w, h), area, true, (left, top));
        }
    }
}

/// Let go. The pointer is taken off whatever it is over, so that is told it has come back
/// and sets its own cursor again.
pub fn end(state: &mut State) {
    let Some(d) = state.drag.take() else { return };
    // Where the window has ended up is where it is from now on, whatever size it settled at.
    if d.resize.is_some()
        && let Some((r, area)) = place(state, d.id)
    {
        let size = state.layout.float(d.id).size;
        state.layout.set_float(d.id, Float { pos: Some((r.x - area.x, r.y - area.y)), size, hold: (false, false) });
    }
    state.cursor_status = CursorImageStatus::default_named();
    if let Some(pointer) = state.seat.get_pointer() {
        let time = smithay::backend::input::InputTime::from_millis(state.start.elapsed().as_millis() as u32);
        pointer.motion(state, None, &MotionEvent { location: state.pointer_pos, serial: SERIAL_COUNTER.next_serial(), time });
        pointer.frame(state);
    }
    state.queue_redraw();
}

/// The keys that move and size columns do the same to a floating window with the focus.
/// True when the action was one of those, and is done.
pub fn action(state: &mut State, action: &Action) -> bool {
    let Some(id) = state.layout.active_window().filter(|id| state.layout.is_floating(*id)) else { return false };
    let Some((r, area)) = place(state, id) else { return false };
    let part = |of: i32, change: &WidthChange, now: i32| match change {
        WidthChange::By(b) => now + (of as f64 * b).round() as i32,
        WidthChange::To(t) => (of as f64 * t).round() as i32,
    };
    match action {
        Action::MoveColumn(d) => {
            let (dx, dy) = match d {
                Dir::Left => (-STEP, 0),
                Dir::Right => (STEP, 0),
                Dir::Up => (0, -STEP),
                Dir::Down => (0, STEP),
            };
            put(state, id, Rect::new(r.x + dx, r.y + dy, r.w, r.h), area, false, (false, false));
        }
        Action::SetColumnWidth(c) => put(state, id, Rect::new(r.x, r.y, part(area.w, c, r.w), r.h), area, true, (false, false)),
        Action::SetWindowHeight(c) => put(state, id, Rect::new(r.x, r.y, r.w, part(area.h, c, r.h)), area, true, (false, false)),
        // Back to the size it chooses itself, and back to the middle.
        Action::ResetWindowHeight => state.layout.set_float(id, Float { pos: state.layout.float(id).pos, size: None, hold: (false, false) }),
        Action::CenterColumn => state.layout.set_float(id, Float { pos: None, size: state.layout.float(id).size, hold: (false, false) }),
        // These are about columns; a floating window has none.
        Action::SwitchPresetWidth | Action::MaximizeColumn | Action::ConsumeOrExpel(_) => {}
        _ => return false,
    }
    true
}
