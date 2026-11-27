//! Windows moved and resized with the pointer. Mod and the left button moves a floating
//! window, or picks one of the columns up; Mod and the right button resizes a floating
//! window from the nearest corner, or makes a column wider or narrower. A floating window's
//! own title bar and edges work too, when it has them, and so do the keys that move and
//! size columns.
//!
//! With `snap` on, a floating window being moved lines its edges up with the room's and with
//! other floating windows' when they come close, and the pointer at an edge of the screen
//! shows where it would go if let go of there: half the room by a side, a quarter by a
//! corner, all of it at the top.

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
    /// down): how wide it was, how many pixels the whole screen is, and whether it runs down,
    /// and whether it is the near edge (the left, or the top) that follows the pointer.
    column: Option<(f64, f64, bool, bool)>,
    pub spot: Option<(Spot, String, Rect)>,
    /// A floating window carried onto another display: let go of, all of it goes on that one.
    crossed: bool,
    /// Where a floating window would snap to if let go of now, on the display named.
    pub snap: Option<(String, Rect)>,
    /// A snapped window, still to be given back its size once it is carried away.
    unsnap: bool,
    /// Where a floating window was and the size it had before it was picked up (before it
    /// was snapped, for one that was): what snapping it now goes back to.
    was: (Option<(i32, i32)>, Option<(i32, i32)>),
    pub mark: crate::render::Deco,
    pub mark_id: smithay::backend::renderer::element::Id,
}

/// How far a picked-up window is carried before it counts as being moved.
const CARRY: f64 = 12.0;

/// How close an edge comes to another before it is pulled onto it.
const SNAP: i32 = 14;

/// How near the pointer is to the edge of the screen for the window to snap to that side.
const EDGE: f64 = 3.0;

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
    let output = state.layout.output_of(id).and_then(|n| state.output_named(n))?;
    let rect = state.tiles(output).into_iter().find(|t| t.id == id && !t.fullscreen)?.rect;
    Some((rect, area))
}

/// Where the display a window is on has its corner, among all of them.
fn origin(state: &State, id: WinId) -> Option<Point<i32, Logical>> {
    state.layout.output_of(id).and_then(|n| state.output_named(n)).map(|o| State::output_geometry(o).loc)
}

fn put(state: &mut State, id: WinId, rect: Rect, area: Rect, sized: bool, hold: (bool, bool)) {
    let size = if sized { Some((rect.w, rect.h)) } else { state.layout.float(id).size };
    state.layout.set_float(id, Float { pos: Some((rect.x - area.x, rect.y - area.y)), size, hold, before: None });
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
            let Some((was, whole, down)) = state.layout.column_width_of(id).filter(|_| !state.overview.on && !state.layout.is_floating(id)) else { return false };
            // The edge nearer the pointer is the one that moves.
            let near = state.wins.get(&id).and_then(|w| w.output.as_deref()).and_then(|n| state.output_named(n)).and_then(|o| {
                let origin = State::output_geometry(o).loc;
                let r = state.tiles(o).into_iter().find(|t| t.id == id)?.rect;
                Some(if down { from.y < (origin.y + r.y) as f64 + r.h as f64 / 2.0 } else { from.x < (origin.x + r.x) as f64 + r.w as f64 / 2.0 })
            });
            let column = (was, whole, down, near.unwrap_or(false));
            state.cursor_status = CursorImageStatus::Named(if column.2 { CursorIcon::NsResize } else { CursorIcon::EwResize });
            state.drag = Some(Drag { id, resize: None, from, rect: Rect::new(0, 0, 0, 0), quiet, tiled: None, column: Some(column), spot: None, crossed: false, snap: None, unsnap: false, was: (None, None), mark: Default::default(), mark_id: smithay::backend::renderer::element::Id::new() });
            state.queue_redraw();
            return true;
        }
        // A window of the columns: picked up, to be put down at another place among them.
        let tile = state.wins.get(&id).and_then(|w| w.output.as_deref()).and_then(|n| state.output_named(n)).and_then(|o| {
            let origin = State::output_geometry(o).loc;
            state.tiles(o).into_iter().find(|t| t.id == id && !t.floating && !t.fullscreen && !t.hidden).map(|t| (t, Rect::new(t.rect.x + origin.x, t.rect.y + origin.y, t.rect.w, t.rect.h)))
        });
        let Some((tile, rect)) = tile.filter(|_| !resize && !state.overview.on) else { return false };
        state.drag = Some(Drag { id, resize: None, from, rect, quiet, tiled: Some(tile), column: None, spot: None, crossed: false, snap: None, unsnap: false, was: (None, None), mark: Default::default(), mark_id: smithay::backend::renderer::element::Id::new() });
        return true;
    };
    // Held among all the displays, so it can be carried from one to another.
    let at = origin(state, id).unwrap_or_default();
    let rect = Rect::new(rect.x + at.x, rect.y + at.y, rect.w, rect.h);
    let resize = resize.then(|| corner.unwrap_or_else(|| (from.x < (rect.x + rect.w / 2) as f64, from.y < (rect.y + rect.h / 2) as f64)));
    state.cursor_status = CursorImageStatus::Named(match resize {
        None => CursorIcon::Grabbing,
        Some((true, true)) => CursorIcon::NwResize,
        Some((false, true)) => CursorIcon::NeResize,
        Some((true, false)) => CursorIcon::SwResize,
        Some((false, false)) => CursorIcon::SeResize,
    });
    let f = state.layout.float(id);
    let unsnap = resize.is_none() && f.before.is_some();
    let was = f.before.unwrap_or((f.pos, f.size));
    state.drag = Some(Drag { id, resize, from, rect, quiet, tiled: None, column: None, spot: None, crossed: false, snap: None, unsnap, was, mark: Default::default(), mark_id: smithay::backend::renderer::element::Id::new() });
    state.queue_redraw();
    true
}

/// A floating window's edges pulled onto the room's edges, and onto other floating windows'
/// (or a gap away from them), where they are near. `r` is the window on its display.
fn magnet(state: &State, id: WinId, r: Rect, area: Rect) -> Rect {
    let o = &state.layout.opts;
    let (b, g) = (o.border, o.gaps);
    let outside = |r: Rect, bar: i32| Rect::new(r.x - b, r.y - b - bar, r.w + 2 * b, r.h + 2 * b + bar);
    let me = outside(r, state.layout.bar_for(id, true));
    let room = state.layout.room(area);
    let (mut xs, mut ys) = (vec![room.x, room.x + room.w], vec![room.y, room.y + room.h]);
    if let Some(output) = state.layout.output_of(id).and_then(|n| state.output_named(n)) {
        for t in state.tiles(output).iter().filter(|t| t.floating && t.id != id && !t.hidden) {
            let w = outside(t.rect, t.bar);
            xs.extend([w.x, w.x + w.w, w.x - g, w.x + w.w + g]);
            ys.extend([w.y, w.y + w.h, w.y - g, w.y + w.h + g]);
        }
    }
    let pull = |at: i32, len: i32, lines: &[i32]| lines.iter().flat_map(|l| [l - at, l - at - len]).filter(|d| d.abs() <= SNAP).min_by_key(|d| d.abs()).unwrap_or(0);
    Rect::new(r.x + pull(me.x, me.w, &xs), r.y + pull(me.y, me.h, &ys), r.w, r.h)
}

/// What the window would fill if let go of with the pointer where it is: half the room by a
/// side of the screen, a quarter by a corner, all of it at the top. On the display named,
/// as its area is.
fn snap_zone(state: &State) -> Option<(String, Rect)> {
    let output = state.output_at(state.pointer_pos)?;
    let geo = State::output_geometry(output);
    let (x, y) = (state.pointer_pos.x - geo.loc.x as f64, state.pointer_pos.y - geo.loc.y as f64);
    let (w, h) = (geo.size.w as f64, geo.size.h as f64);
    let corner = (w.min(h) / 6.0).max(40.0);
    let (left, right, top, bottom) = (x <= EDGE, x >= w - 1.0 - EDGE, y <= EDGE, y >= h - 1.0 - EDGE);
    let (near_left, near_right, near_top, near_bottom) = (x < corner, x > w - corner, y < corner, y > h - corner);
    let name = output.name();
    let room = state.layout.room(state.layout.mons.get(state.layout.monitor(&name)?)?.area);
    let g = state.layout.opts.gaps;
    let (hw, hh) = ((room.w - g) / 2, (room.h - g) / 2);
    let (r, b) = (room.x + room.w - hw, room.y + room.h - hh);
    let to = if (left && near_top) || (top && near_left) {
        Rect::new(room.x, room.y, hw, hh)
    } else if (right && near_top) || (top && near_right) {
        Rect::new(r, room.y, hw, hh)
    } else if (left && near_bottom) || (bottom && near_left) {
        Rect::new(room.x, b, hw, hh)
    } else if (right && near_bottom) || (bottom && near_right) {
        Rect::new(r, b, hw, hh)
    } else if left {
        Rect::new(room.x, room.y, hw, room.h)
    } else if right {
        Rect::new(r, room.y, hw, room.h)
    } else if top {
        room
    } else {
        return None;
    };
    Some((name, to))
}

/// The pointer moved while a window is held.
pub fn moved(state: &mut State) {
    let Some(d) = &state.drag else { return };
    let (id, r, resize) = (d.id, d.rect, d.resize);
    if let Some((was, whole, down, near)) = d.column {
        let by = if down { state.pointer_pos.y - d.from.y } else { state.pointer_pos.x - d.from.x };
        state.layout.set_column_width_of(id, was + if near { -by } else { by } / whole, near);
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
    if !state.layout.is_floating(id) {
        end(state);
        return;
    }
    // Carried away from where it snapped to, it has its own size back, held where it was by
    // the pointer.
    let mut r = r;
    if d.unsnap && (dx as f64).hypot(dy as f64) >= CARRY {
        let from = d.from;
        if let Some(size) = state.layout.unsnap(id) {
            let (w, h) = size.unwrap_or((r.w, r.h));
            let part = ((from.x - r.x as f64) / r.w.max(1) as f64).clamp(0.0, 1.0);
            r = Rect::new((from.x - part * w as f64).round() as i32, r.y, w, h);
        }
        if let Some(d) = &mut state.drag {
            d.unsnap = false;
            d.rect = r;
        }
    }
    // Carried onto another display, it goes to that display, with the focus.
    if resize.is_none()
        && let Some(name) = state.output_at(state.pointer_pos).map(|o| o.name()).filter(|n| state.layout.output_of(id) != Some(n.as_str()))
    {
        let wins = &state.wins;
        state.layout.move_window_to_monitor(id, &name, &|w| State::chosen_size(wins, w));
        if let Some(d) = &mut state.drag {
            d.crossed = true;
        }
    }
    let (Some(area), Some(at)) = (state.layout.area_of(id), origin(state, id)) else {
        end(state);
        return;
    };
    match resize {
        None => {
            let mut to = Rect::new(r.x + dx - at.x, r.y + dy - at.y, r.w, r.h);
            if state.config.snap {
                to = magnet(state, id, to, area);
                let zone = snap_zone(state);
                if let Some(d) = &mut state.drag {
                    d.snap = zone;
                }
            }
            put(state, id, to, area, false, (false, false));
        }
        Some((left, top)) => {
            let w = (if left { r.w - dx } else { r.w + dx }).max(80);
            let h = (if top { r.h - dy } else { r.h + dy }).max(60);
            // The corner across from the one held stays where it is.
            let x = if left { r.x + r.w - w } else { r.x };
            let y = if top { r.y + r.h - h } else { r.y };
            put(state, id, Rect::new(x - at.x, y - at.y, w, h), area, true, (left, top));
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
    if d.crossed
        && let Some((r, area)) = place(state, d.id)
    {
        let pos = ((r.x - area.x).clamp(0, (area.w - r.w).max(0)), (r.y - area.y).clamp(0, (area.h - r.h).max(0)));
        let f = state.layout.float(d.id);
        state.layout.set_float(d.id, Float { pos: Some(pos), ..f });
    }
    // Let go of at an edge of the screen: it fills that part of the room.
    if let Some((_, to)) = d.snap {
        state.layout.set_float(d.id, Float { pos: d.was.0, size: d.was.1, ..Default::default() });
        state.layout.snap_float(d.id, to);
    }
    // Where the window has ended up is where it is from now on, whatever size it settled at.
    if d.resize.is_some()
        && let Some((r, area)) = place(state, d.id)
    {
        let size = state.layout.float(d.id).size;
        state.layout.set_float(d.id, Float { pos: Some((r.x - area.x, r.y - area.y)), size, ..Default::default() });
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
        Action::ResetWindowHeight => state.layout.set_float(id, Float { pos: state.layout.float(id).pos, ..Default::default() }),
        Action::CenterColumn => state.layout.set_float(id, Float { size: state.layout.float(id).size, ..Default::default() }),
        Action::MaximizeColumn => state.layout.toggle_float_maximized(id),
        // On a workspace of floating windows only, the focus goes to the nearest one that way.
        Action::Focus(d) if state.layout.active_workspace().is_some_and(|w| w.cols.is_empty()) => {
            let Some(output) = state.layout.output_of(id).and_then(|n| state.output_named(n)) else { return true };
            let mid = |r: &Rect| (r.x + r.w / 2, r.y + r.h / 2);
            let (cx, cy) = mid(&r);
            let next = state.tiles(output).into_iter().filter(|t| t.floating && t.id != id && !t.hidden).filter_map(|t| {
                let (x, y) = mid(&t.rect);
                let (along, across) = match d {
                    Dir::Left => (cx - x, y - cy),
                    Dir::Right => (x - cx, y - cy),
                    Dir::Up => (cy - y, x - cx),
                    Dir::Down => (y - cy, x - cx),
                };
                (along > 0).then_some((along + 2 * across.abs(), t.id))
            });
            if let Some((_, to)) = next.min() {
                state.layout.focus_window(to);
            }
        }
        // These are about columns; a floating window has none.
        Action::SwitchPresetWidth | Action::ConsumeOrExpel(_) => {}
        _ => return false,
    }
    true
}
