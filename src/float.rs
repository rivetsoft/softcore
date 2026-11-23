//! Windows moved and resized with the pointer. Mod and the left button moves a floating
//! window, or picks one of the columns up; Mod and the right button resizes a floating
//! window from the nearest corner, or makes a column wider or narrower. A floating window's
//! own title bar and edges work too, when it has them, and so do the keys that move and
//! size columns.

use smithay::input::pointer::{CursorIcon, CursorImageStatus, MotionEvent};
use smithay::utils::{Logical, Point, SERIAL_COUNTER};

use crate::config::{Action, WidthChange};
use crate::layout::{Dir, Float, Rect, Spot, Tile, WinId};
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
    /// A window of the columns, picked up to be put down somewhere else among them: how it
    /// was when picked up, and, once it has been carried a little way, where it would go
    /// if let go of now and the mark that shows it (on the display named).
    pub tiled: Option<Tile>,
    /// A column being made wider or narrower (a row taller or shorter, where the strip runs
    /// down): how wide it was, how many pixels the whole screen is, and whether it runs down.
    column: Option<(f64, f64, bool)>,
    pub spot: Option<(Spot, String, Rect)>,
    pub mark: crate::render::Deco,
    pub mark_id: smithay::backend::renderer::element::Id,
}

/// How far a picked-up window is carried before it counts as being moved.
const CARRY: f64 = 12.0;

/// How small a carried window is drawn, so what is under it shows.
const CARRIED: f64 = 0.5;

/// A carried window as it is drawn on a display: small, by the pointer where it was held.
/// The window is taken out of `tiles`, where it stood.
pub fn carried(state: &State, output: &smithay::output::Output, tiles: &mut Vec<Tile>) -> Option<Tile> {
    let d = state.drag.as_ref()?;
    let t = d.tiled.filter(|_| d.spot.is_some())?;
    tiles.retain(|x| x.id != t.id);
    let geo = State::output_geometry(output);
    if !geo.to_f64().contains(state.pointer_pos) {
        return None;
    }
    // The point it was held by stays under the pointer.
    let (px, py) = (state.pointer_pos.x - geo.loc.x as f64, state.pointer_pos.y - geo.loc.y as f64);
    let (hx, hy) = (d.from.x - d.rect.x as f64, d.from.y - d.rect.y as f64);
    let rect = Rect::new((px - hx * CARRIED).round() as i32, (py - hy * CARRIED).round() as i32, ((t.rect.w as f64 * CARRIED).round() as i32).max(1), ((t.rect.h as f64 * CARRIED).round() as i32).max(1));
    Some(Tile { rect, zoom: CARRIED, active: true, moving: false, resizing: false, tab: None, hidden: false, ..t })
}

/// Where a carried window would go if let go of now, and the mark for it: a bar between
/// columns for a column of its own, half of a window for a place in that window's column.
fn spot_at(state: &State, id: WinId) -> Option<(Spot, String, Rect)> {
    let output = state.output_at(state.pointer_pos)?;
    let name = output.name();
    let mi = state.layout.monitor(&name)?;
    let m = &state.layout.mons[mi];
    let o = &state.layout.opts;
    let vertical = m.wss.get(m.active).is_some_and(|ws| m.ws_vertical(ws, o));
    let geo = State::output_geometry(output);
    // Everything as if the strip ran across: turned for one that runs down.
    let turn = |r: Rect| if vertical { Rect::new(r.y, r.x, r.h, r.w) } else { r };
    let (px, py) = ((state.pointer_pos.x - geo.loc.x as f64).round() as i32, (state.pointer_pos.y - geo.loc.y as f64).round() as i32);
    let (px, py) = if vertical { (py, px) } else { (px, py) };
    let area = turn(m.area);
    let tiles: Vec<(WinId, Rect)> = state.tiles(output).iter().filter(|t| !t.floating && !t.fullscreen && !t.hidden && t.id != id).map(|t| (t.id, turn(t.rect))).collect();
    let (top, tall) = (area.y + o.gaps_out, (area.h - 2 * o.gaps_out).max(1));
    let Some((near, r)) = tiles.iter().copied().min_by_key(|(_, r)| {
        let dx = (r.x - px).max(px - (r.x + r.w)).max(0);
        let dy = (r.y - py).max(py - (r.y + r.h)).max(0);
        dx * dx + dy * dy
    }) else {
        return Some((Spot::Last(mi), name, turn(Rect::new(area.x + o.gaps_out, top, (area.w - 2 * o.gaps_out).max(1), tall))));
    };
    // The outer quarters of a column are for a column beside it, the middle for a place in it.
    let edge = (r.w / 4).max(1);
    let thick = o.gaps.max(8);
    let reach = o.border + o.gaps / 2;
    let (spot, mark) = if px < r.x + edge {
        (Spot::Before(near), Rect::new(r.x - reach - thick / 2, top, thick, tall))
    } else if px >= r.x + r.w - edge {
        (Spot::After(near), Rect::new(r.x + r.w + reach - thick / 2, top, thick, tall))
    } else if py < r.y + r.h / 2 {
        (Spot::Above(near), Rect::new(r.x, r.y, r.w, r.h / 2))
    } else {
        (Spot::Below(near), Rect::new(r.x, r.y + r.h / 2, r.w, r.h - r.h / 2))
    };
    Some((spot, name, turn(mark)))
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
    let from = state.pointer_pos;
    let Some((rect, _)) = place(state, id) else {
        // A window of the columns, by the right button: its column is made wider or narrower
        // as the pointer goes along the strip.
        if resize {
            let Some(column) = state.layout.column_width_of(id).filter(|_| !state.overview.on && !state.layout.is_floating(id)) else { return false };
            state.cursor_status = CursorImageStatus::Named(if column.2 { CursorIcon::NsResize } else { CursorIcon::EwResize });
            state.drag = Some(Drag { id, resize: None, from, rect: Rect::new(0, 0, 0, 0), quiet, tiled: None, column: Some(column), spot: None, mark: Default::default(), mark_id: smithay::backend::renderer::element::Id::new() });
            state.queue_redraw();
            return true;
        }
        // A window of the columns: picked up, to be put down at another place among them.
        let tile = state.wins.get(&id).and_then(|w| w.output.as_deref()).and_then(|n| state.output_named(n)).and_then(|o| {
            let origin = State::output_geometry(o).loc;
            state.tiles(o).into_iter().find(|t| t.id == id && !t.floating && !t.fullscreen && !t.hidden).map(|t| (t, Rect::new(t.rect.x + origin.x, t.rect.y + origin.y, t.rect.w, t.rect.h)))
        });
        let Some((tile, rect)) = tile.filter(|_| !resize && !state.overview.on) else { return false };
        state.drag = Some(Drag { id, resize: None, from, rect, quiet, tiled: Some(tile), column: None, spot: None, mark: Default::default(), mark_id: smithay::backend::renderer::element::Id::new() });
        return true;
    };
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
    state.drag = Some(Drag { id, resize, from, rect, quiet, tiled: None, column: None, spot: None, mark: Default::default(), mark_id: smithay::backend::renderer::element::Id::new() });
    state.queue_redraw();
    true
}

/// The pointer moved while a window is held.
pub fn moved(state: &mut State) {
    let Some(d) = &state.drag else { return };
    let (id, r, resize) = (d.id, d.rect, d.resize);
    if let Some((was, whole, down)) = d.column {
        let by = if down { state.pointer_pos.y - d.from.y } else { state.pointer_pos.x - d.from.x };
        state.layout.set_column_width_of(id, was + by / whole);
        return;
    }
    if d.tiled.is_some() {
        let far = (state.pointer_pos.x - d.from.x).hypot(state.pointer_pos.y - d.from.y) >= CARRY;
        if d.spot.is_some() || far {
            if d.spot.is_none() {
                state.cursor_status = CursorImageStatus::Named(CursorIcon::Grabbing);
            }
            let spot = spot_at(state, id);
            if let Some(d) = &mut state.drag {
                // Off every display there is nowhere to go: the last place shown stands.
                d.spot = spot.or(d.spot.take());
            }
        }
        return;
    }
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
    if let Some((spot, ..)) = d.spot {
        state.layout.drop_window(d.id, spot);
        state.layer_focus = None;
        state.update_focus();
    }
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
