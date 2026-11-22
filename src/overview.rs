//! The overview: every workspace of a display at once, drawn small. Windows are found and
//! picked with the pointer or the keys, and the usual bindings go on working in it.

use std::collections::HashMap;

use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::{Id, Kind};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::output::Output;
use smithay::utils::{Logical, Physical, Point, Rectangle, Scale};

use crate::layout::{Monitor, Rect, Tile};
use crate::render::Elem;
use crate::state::State;

#[derive(Default)]
pub struct Overview {
    pub on: bool,
    /// From 0, the usual view, to 1, when everything has drawn back.
    pub p: f64,
    /// The click that picked something: the button's letting go is not for whatever is under
    /// the pointer by then.
    pub eat_release: bool,
    /// What is drawn behind each workspace, kept while it looks the same.
    plates: HashMap<(String, u64), (Id, [u8; 4])>,
}

/// The room between two workspaces, as a part of the screen.
const BETWEEN: f64 = 0.08;

/// Where things on a display's workspaces are drawn while the overview is (partly) open.
pub struct Map {
    zoom: f64,
    centre: (f64, f64),
    /// From one workspace to the next, before it is all made smaller.
    step: (f64, f64),
    pos: f64,
}

impl Map {
    /// A rectangle of a workspace (by its place in the stack), as it is on the screen.
    pub fn place(&self, ws: usize, r: Rect) -> Rect {
        let by = ws as f64 - self.pos;
        let at = |v: f64, step: f64, c: f64| (c + (v + by * step - c) * self.zoom).round() as i32;
        let (x0, y0) = (at(r.x as f64, self.step.0, self.centre.0), at(r.y as f64, self.step.1, self.centre.1));
        let (x1, y1) = (at((r.x + r.w) as f64, self.step.0, self.centre.0), at((r.y + r.h) as f64, self.step.1, self.centre.1));
        Rect::new(x0, y0, (x1 - x0).max(1), (y1 - y0).max(1))
    }
}

fn ease(p: f64) -> f64 {
    1.0 - (1.0 - p.clamp(0.0, 1.0)).powi(3)
}

impl State {
    /// None with the overview shut, and everything where it usually is.
    pub fn overview_map(&self, m: &Monitor) -> Option<Map> {
        if self.overview.p <= 0.0 {
            return None;
        }
        let e = ease(self.overview.p);
        let (w, h) = (m.size.0 as f64, m.size.1 as f64);
        // Workspaces go the other way from the windows.
        let step = if m.vertical(&self.layout.opts) { (w * (1.0 + BETWEEN * e), 0.0) } else { (0.0, h * (1.0 + BETWEEN * e)) };
        Some(Map { zoom: 1.0 + (self.config.overview_zoom - 1.0) * e, centre: (w / 2.0, h / 2.0), step, pos: m.pos })
    }

    /// The windows of every workspace of a display, drawn back.
    pub fn overview_tiles(&self, mi: usize, map: &Map) -> Vec<Tile> {
        let m = &self.layout.mons[mi];
        let screen = Rect::new(0, 0, m.size.0, m.size.1);
        let mut out: Vec<Tile> = self
            .layout
            .all_tiles(mi, &self.float_size())
            .into_iter()
            .map(|(ws, t)| Tile { rect: map.place(ws, t.rect), tab: t.tab.map(|r| map.place(ws, r)), zoom: map.zoom, border: t.border && !t.fullscreen, fullscreen: false, ..t })
            .collect();
        out.retain(|t| t.rect.intersects(&screen));
        out
    }

    pub fn set_overview(&mut self, on: bool) {
        if self.overview.on != on {
            self.overview.on = on;
            if !self.config.animations {
                self.overview.p = if on { 1.0 } else { 0.0 };
            }
            self.update_focus();
            self.queue_redraw();
        }
    }
}

/// A click in the overview, on no panel of the shell's: the window under it gets the focus,
/// or else the workspace under it comes into view; and the overview shuts.
pub fn clicked(state: &mut State, pos: Point<f64, Logical>) {
    state.overview.eat_release = true;
    if let Some(output) = state.output_at(pos).cloned() {
        let local = pos - State::output_geometry(&output).loc.to_f64();
        let inside = |r: &Rect| local.x >= r.x as f64 && local.x < (r.x + r.w) as f64 && local.y >= r.y as f64 && local.y < (r.y + r.h) as f64;
        state.layout.set_active_monitor(&output.name());
        if let Some(t) = state.tiles(&output).iter().rev().find(|t| !t.hidden && inside(&t.rect)) {
            state.layout.focus_window(t.id);
        } else if let Some(mi) = state.layout.monitor(&output.name()) {
            let m = &state.layout.mons[mi];
            let whole = Rect::new(0, 0, m.size.0, m.size.1);
            let hit = state.overview_map(m).and_then(|map| (0..m.wss.len()).find(|wi| inside(&map.place(*wi, whole))));
            if let Some(wi) = hit {
                state.layout.focus_workspace(wi);
            }
        }
    }
    state.layer_focus = None;
    state.set_overview(false);
}

/// What is behind the windows in the overview: the wallpaper dimmed, and each workspace a
/// lighter sheet on it, the one in view lighter still.
pub fn backdrop(state: &mut State, output: &Output, scale: Scale<f64>, out: &mut Vec<Elem>) {
    let name = output.name();
    let Some(mi) = state.layout.monitor(&name) else { return };
    let m = &state.layout.mons[mi];
    let Some(map) = state.overview_map(m) else {
        state.overview.plates.retain(|k, _| k.0 != name);
        return;
    };
    let e = ease(state.overview.p);
    let whole = Rect::new(0, 0, m.size.0, m.size.1);
    let mut sheets: Vec<(u64, Rect, [u8; 4])> = m.wss.iter().enumerate().map(|(wi, ws)| (ws.id, map.place(wi, whole), [255, 255, 255, ((if wi == m.active { 30.0 } else { 16.0 }) * e) as u8])).filter(|s| s.1.intersects(&whole)).collect();
    // 0 is no workspace's number: the dimming over the whole display.
    sheets.push((0, whole, [0, 0, 0, (110.0 * e) as u8]));
    state.overview.plates.retain(|k, _| k.0 != name || sheets.iter().any(|s| s.0 == k.1));
    for (id, r, color) in sheets {
        let kept = state.overview.plates.entry((name.clone(), id)).or_insert_with(|| (Id::new(), color));
        if kept.1 != color {
            *kept = (Id::new(), color);
        }
        let a = color[3] as f32 / 255.0;
        let rgba = [color[0] as f32 / 255.0 * a, color[1] as f32 / 255.0 * a, color[2] as f32 / 255.0 * a, a];
        let geo: Rectangle<i32, Physical> = Rectangle::<i32, Logical>::new((r.x, r.y).into(), (r.w, r.h).into()).to_physical_precise_round(scale);
        out.push(Elem::Solid(SolidColorRenderElement::new(kept.0.clone(), geo, CommitCounter::default(), rgba, Kind::Unspecified)));
    }
}
