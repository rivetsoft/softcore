//! The scrolling layout: windows sit in columns on a strip that runs off both sides of the
//! screen, and each display has its own stack of workspaces, with an empty one always last.
//!
//! Nothing here knows about Wayland. Windows are numbers, rectangles are in a display's own
//! logical pixels, and the compositor asks for `tiles()` each frame. That keeps every rule of
//! the layout testable without a screen.

use std::collections::HashMap;

pub type WinId = u32;

/// A rectangle in a display's logical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn intersects(&self, o: &Rect) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }
}

pub use softcore_settings::{Dir, Options};

#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub wins: Vec<WinId>,
    pub active: usize,
    /// Part of the screen's width.
    pub width: f64,
    /// As wide as the screen allows, until toggled back.
    pub maximized: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Workspace {
    pub id: u64,
    pub cols: Vec<Column>,
    pub active: usize,
    /// Where the strip is scrolled to now, and where it is heading.
    pub view: f64,
    pub view_to: f64,
    /// Windows that float over the columns (dialogs, fixed-size windows), bottom first.
    pub floating: Vec<WinId>,
    /// The floating window that has the focus, when one does.
    pub float_focus: Option<WinId>,
    pub fullscreen: Option<WinId>,
    /// The display it belongs on: where it goes back to when that display returns after
    /// being away. An empty workspace belongs wherever it is.
    pub home: String,
}

impl Workspace {
    fn new(id: u64) -> Workspace {
        Workspace { id, cols: Vec::new(), active: 0, view: 0.0, view_to: 0.0, floating: Vec::new(), float_focus: None, fullscreen: None, home: String::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.cols.is_empty() && self.floating.is_empty()
    }

    pub fn active_window(&self) -> Option<WinId> {
        if let Some(f) = self.fullscreen {
            return Some(f);
        }
        if let Some(f) = self.float_focus {
            return Some(f);
        }
        self.cols.get(self.active).and_then(|c| c.wins.get(c.active)).copied()
    }

    pub fn windows(&self) -> impl Iterator<Item = WinId> + '_ {
        self.cols.iter().flat_map(|c| c.wins.iter().copied()).chain(self.floating.iter().copied())
    }

    fn col_width(&self, i: usize, area: &Rect, o: &Options) -> i32 {
        let c = &self.cols[i];
        let full = area.w - 2 * o.gaps_out;
        if c.maximized {
            return full.max(1);
        }
        // Two halves, or three thirds, fill the screen exactly with the gaps between them.
        ((((full + o.gaps) as f64) * c.width).round() as i32 - o.gaps).clamp(1, full.max(1))
    }

    /// A column's left edge on the strip.
    fn col_x(&self, i: usize, area: &Rect, o: &Options) -> i32 {
        (0..i).map(|j| self.col_width(j, area, o) + o.gaps).sum()
    }

    /// Scroll just far enough that the active column is whole on screen.
    fn settle_view(&mut self, area: &Rect, o: &Options) {
        if self.cols.is_empty() {
            self.active = 0;
            self.view_to = 0.0;
            return;
        }
        self.active = self.active.min(self.cols.len() - 1);
        for c in &mut self.cols {
            c.active = c.active.min(c.wins.len().saturating_sub(1));
        }
        let x = self.col_x(self.active, area, o) as f64;
        let w = self.col_width(self.active, area, o) as f64;
        let room = (area.w - 2 * o.gaps_out) as f64;
        if x < self.view_to || w >= room {
            self.view_to = x;
        } else if x + w > self.view_to + room {
            self.view_to = x + w - room;
        }
        // Nothing to the right to show: don't leave a hole on the left.
        let last = self.cols.len() - 1;
        let end = (self.col_x(last, area, o) + self.col_width(last, area, o)) as f64;
        if end <= room {
            self.view_to = 0.0;
        }
    }

    fn remove(&mut self, id: WinId) -> bool {
        if self.fullscreen == Some(id) {
            self.fullscreen = None;
        }
        if let Some(i) = self.floating.iter().position(|w| *w == id) {
            self.floating.remove(i);
            if self.float_focus == Some(id) {
                self.float_focus = if self.cols.is_empty() { self.floating.last().copied() } else { None };
            }
            return true;
        }
        for ci in 0..self.cols.len() {
            if let Some(wi) = self.cols[ci].wins.iter().position(|w| *w == id) {
                let col = &mut self.cols[ci];
                col.wins.remove(wi);
                if wi < col.active || col.active >= col.wins.len() {
                    col.active = col.active.saturating_sub(1);
                }
                if col.wins.is_empty() {
                    self.cols.remove(ci);
                    // Closing a column leaves the focus on the one that slid into its place's
                    // left, like closing a tab.
                    if ci < self.active || (ci == self.active && ci > 0) {
                        self.active -= 1;
                    }
                }
                if self.cols.is_empty() && self.float_focus.is_none() {
                    self.float_focus = self.floating.last().copied();
                }
                return true;
            }
        }
        false
    }

    fn insert_column(&mut self, col: Column) {
        let at = if self.cols.is_empty() { 0 } else { self.active + 1 };
        self.cols.insert(at, col);
        self.active = at;
        self.float_focus = None;
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    pub name: String,
    pub wss: Vec<Workspace>,
    pub active: usize,
    /// The workspace shown, as a number that slides from one to the next.
    pub pos: f64,
    /// The whole display, and the part panels leave free.
    pub size: (i32, i32),
    pub area: Rect,
    /// Its display is gone; the next one to appear takes its workspaces.
    pub orphan: bool,
}

/// Where a window is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tile {
    pub id: WinId,
    /// The window itself (the border goes around it).
    pub rect: Rect,
    pub border: bool,
    /// The focused window of its workspace.
    pub active: bool,
    pub floating: bool,
    pub fullscreen: bool,
    /// On its way to its place: drawn at this size whatever size the window itself has.
    pub moving: bool,
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub mons: Vec<Monitor>,
    pub active_mon: usize,
    pub opts: Options,
    next_ws: u64,
    /// Where each tiled window stood on its strip after the last change, and its workspace.
    places: HashMap<WinId, (u64, Rect)>,
    /// How far each window still is from its place (x, y, width, height), shrinking to nothing.
    offsets: HashMap<WinId, [f64; 4]>,
    /// Windows given a height of their own, as a part of the height a column has. The
    /// others share what is left.
    heights: HashMap<WinId, f64>,
    /// Windows put away in the scratchpad, the next one to show first.
    pub scratch: Vec<WinId>,
    /// Windows out of the scratchpad and on show: the scratchpad key puts these away again.
    shown: Vec<WinId>,
}

impl Layout {
    pub fn new(opts: Options) -> Layout {
        Layout { mons: Vec::new(), active_mon: 0, opts, next_ws: 1, places: HashMap::new(), offsets: HashMap::new(), heights: HashMap::new(), scratch: Vec::new(), shown: Vec::new() }
    }

    fn new_ws(&mut self) -> Workspace {
        let id = self.next_ws;
        self.next_ws += 1;
        Workspace::new(id)
    }

    // ---- displays ----

    pub fn add_monitor(&mut self, name: &str, size: (i32, i32)) {
        let area = Rect::new(0, 0, size.0, size.1);
        if let Some(m) = self.mons.iter_mut().find(|m| m.orphan) {
            m.name = name.to_string();
            m.orphan = false;
            m.size = size;
            m.area = area;
        } else if !self.mons.iter().any(|m| m.name == name) {
            let ws = self.new_ws();
            let mut new = Monitor { name: name.to_string(), wss: vec![ws], active: 0, pos: 0.0, size, area, orphan: false };
            // Back after being away: the workspaces that went to other displays come home.
            for m in &mut self.mons {
                let mut i = 0;
                while i < m.wss.len() {
                    if m.wss[i].home == name && !m.wss[i].is_empty() {
                        let at = new.wss.len() - 1;
                        new.wss.insert(at, m.wss.remove(i));
                        // The one in view stays in view, or the one that took its place.
                        if i < m.active {
                            m.active -= 1;
                            m.pos = (m.pos - 1.0).max(0.0);
                        }
                    } else {
                        i += 1;
                    }
                }
            }
            self.mons.push(new);
        }
        self.settle();
    }

    /// A display went away: its workspaces move to another, or wait for the next one.
    pub fn remove_monitor(&mut self, name: &str) {
        let Some(i) = self.mons.iter().position(|m| m.name == name) else { return };
        if self.mons.len() == 1 {
            self.mons[0].orphan = true;
            return;
        }
        let gone = self.mons.remove(i);
        let to = if i == 0 { 0 } else { i - 1 };
        let m = &mut self.mons[to];
        let at = m.wss.len() - 1;
        for (k, ws) in gone.wss.into_iter().filter(|w| !w.is_empty()).enumerate() {
            m.wss.insert(at + k, ws);
        }
        if self.active_mon >= self.mons.len() || self.active_mon == i {
            self.active_mon = to;
        } else if self.active_mon > i {
            self.active_mon -= 1;
        }
        self.settle();
    }

    pub fn set_geometry(&mut self, name: &str, size: (i32, i32), area: Rect) -> bool {
        let Some(m) = self.mons.iter_mut().find(|m| m.name == name) else { return false };
        if m.size == size && m.area == area {
            return false;
        }
        m.size = size;
        m.area = area;
        self.settle();
        true
    }

    pub fn monitor(&self, name: &str) -> Option<usize> {
        self.mons.iter().position(|m| m.name == name && !m.orphan)
    }

    pub fn active_monitor(&self) -> Option<&Monitor> {
        self.mons.get(self.active_mon)
    }

    pub fn set_active_monitor(&mut self, name: &str) {
        if let Some(i) = self.monitor(name) {
            self.active_mon = i;
        }
    }

    // ---- looking things up ----

    fn ws(&mut self) -> Option<&mut Workspace> {
        let m = self.mons.get_mut(self.active_mon)?;
        m.wss.get_mut(m.active)
    }

    pub fn active_workspace(&self) -> Option<&Workspace> {
        let m = self.mons.get(self.active_mon)?;
        m.wss.get(m.active)
    }

    /// The window that has the focus.
    pub fn active_window(&self) -> Option<WinId> {
        self.active_workspace().and_then(|w| w.active_window())
    }

    /// The display and workspace (by position) a window is on.
    pub fn find(&self, id: WinId) -> Option<(usize, usize)> {
        for (mi, m) in self.mons.iter().enumerate() {
            for (wi, ws) in m.wss.iter().enumerate() {
                if ws.windows().any(|w| w == id) {
                    return Some((mi, wi));
                }
            }
        }
        None
    }

    /// Column and row, both from 1, for a tiled window.
    pub fn position(&self, id: WinId) -> Option<(usize, usize)> {
        let (mi, wi) = self.find(id)?;
        let ws = &self.mons[mi].wss[wi];
        ws.cols.iter().enumerate().find_map(|(c, col)| col.wins.iter().position(|w| *w == id).map(|r| (c + 1, r + 1)))
    }

    pub fn is_floating(&self, id: WinId) -> bool {
        self.find(id).is_some_and(|(mi, wi)| self.mons[mi].wss[wi].floating.contains(&id))
    }

    // ---- windows coming and going ----

    /// A new window: a column of its own, right of the focused one (or floating over them).
    pub fn add_window(&mut self, id: WinId, floating: bool, on: Option<&str>) {
        if let Some(i) = on.and_then(|n| self.monitor(n)) {
            self.active_mon = i;
        }
        let width = self.opts.default_width;
        let Some(ws) = self.ws() else { return };
        if floating {
            ws.floating.push(id);
            ws.float_focus = Some(id);
        } else {
            ws.insert_column(Column { wins: vec![id], active: 0, width, maximized: false });
        }
        self.settle();
    }

    pub fn remove_window(&mut self, id: WinId) {
        self.scratch.retain(|w| *w != id);
        self.shown.retain(|w| *w != id);
        for m in &mut self.mons {
            for ws in &mut m.wss {
                if ws.remove(id) {
                    self.heights.remove(&id);
                    self.settle();
                    return;
                }
            }
        }
    }

    /// Give a window the focus, switching display and workspace to where it is.
    pub fn focus_window(&mut self, id: WinId) {
        let Some((mi, wi)) = self.find(id) else { return };
        self.active_mon = mi;
        let m = &mut self.mons[mi];
        m.active = wi;
        let ws = &mut m.wss[wi];
        if ws.floating.contains(&id) {
            ws.floating.retain(|w| *w != id);
            ws.floating.push(id);
            ws.float_focus = Some(id);
        } else {
            ws.float_focus = None;
            for (ci, col) in ws.cols.iter_mut().enumerate() {
                if let Some(r) = col.wins.iter().position(|w| *w == id) {
                    col.active = r;
                    ws.active = ci;
                }
            }
        }
        if ws.fullscreen.is_some_and(|f| f != id) {
            ws.fullscreen = None;
        }
        self.settle();
    }

    // ---- moving the focus ----

    pub fn focus(&mut self, dir: Dir) {
        let Some(ws) = self.ws() else { return };
        ws.float_focus = None;
        if ws.cols.is_empty() {
            return;
        }
        match dir {
            Dir::Left => ws.active = ws.active.saturating_sub(1),
            Dir::Right => ws.active = (ws.active + 1).min(ws.cols.len() - 1),
            Dir::Up => {
                let c = &mut ws.cols[ws.active];
                c.active = c.active.saturating_sub(1);
            }
            Dir::Down => {
                let c = &mut ws.cols[ws.active];
                c.active = (c.active + 1).min(c.wins.len() - 1);
            }
        }
        self.settle();
    }

    pub fn focus_column_edge(&mut self, last: bool) {
        let Some(ws) = self.ws() else { return };
        ws.float_focus = None;
        ws.active = if last { ws.cols.len().saturating_sub(1) } else { 0 };
        self.settle();
    }

    /// Between the floating windows and the columns under them.
    pub fn switch_floating_focus(&mut self) {
        let Some(ws) = self.ws() else { return };
        ws.float_focus = match ws.float_focus {
            Some(_) if !ws.cols.is_empty() => None,
            Some(f) => Some(f),
            None => ws.floating.last().copied(),
        };
    }

    // ---- moving windows ----

    pub fn move_column(&mut self, dir: Dir) {
        let Some(ws) = self.ws() else { return };
        if ws.cols.is_empty() || ws.float_focus.is_some() {
            return;
        }
        let a = ws.active;
        match dir {
            Dir::Left if a > 0 => {
                ws.cols.swap(a, a - 1);
                ws.active = a - 1;
            }
            Dir::Right if a + 1 < ws.cols.len() => {
                ws.cols.swap(a, a + 1);
                ws.active = a + 1;
            }
            Dir::Up | Dir::Down => {
                let c = &mut ws.cols[a];
                let r = c.active;
                if dir == Dir::Up && r > 0 {
                    c.wins.swap(r, r - 1);
                    c.active = r - 1;
                } else if dir == Dir::Down && r + 1 < c.wins.len() {
                    c.wins.swap(r, r + 1);
                    c.active = r + 1;
                }
            }
            _ => {}
        }
        self.settle();
    }

    /// Into the neighbouring column when the window has a column to itself; out into a
    /// column of its own when it shares one.
    pub fn consume_or_expel(&mut self, dir: Dir) {
        let width = self.opts.default_width;
        let Some(ws) = self.ws() else { return };
        if ws.cols.is_empty() || ws.float_focus.is_some() {
            return;
        }
        let a = ws.active;
        if ws.cols[a].wins.len() > 1 {
            let c = &mut ws.cols[a];
            let id = c.wins.remove(c.active);
            c.active = c.active.min(c.wins.len() - 1);
            let at = if dir == Dir::Left { a } else { a + 1 };
            ws.cols.insert(at, Column { wins: vec![id], active: 0, width, maximized: false });
            ws.active = at;
        } else {
            let to = match dir {
                Dir::Left if a > 0 => a - 1,
                Dir::Right if a + 1 < ws.cols.len() => a + 1,
                _ => return,
            };
            let id = ws.cols[a].wins[0];
            ws.cols[to].wins.push(id);
            ws.cols[to].active = ws.cols[to].wins.len() - 1;
            ws.cols.remove(a);
            ws.active = if to > a { to - 1 } else { to };
        }
        self.settle();
    }

    // ---- sizes ----

    pub fn switch_preset_width(&mut self) {
        let presets = self.opts.presets.clone();
        let Some(ws) = self.ws() else { return };
        let Some(c) = ws.cols.get_mut(ws.active) else { return };
        c.maximized = false;
        let next = presets.iter().copied().find(|p| *p > c.width + 0.005).or(presets.first().copied());
        if let Some(p) = next {
            c.width = p;
        }
        self.settle();
    }

    pub fn change_width(&mut self, by: f64) {
        let Some(ws) = self.ws() else { return };
        let Some(c) = ws.cols.get_mut(ws.active) else { return };
        c.maximized = false;
        c.width = (c.width + by).clamp(0.1, 1.0);
        self.settle();
    }

    pub fn set_width(&mut self, to: f64) {
        let Some(ws) = self.ws() else { return };
        let Some(c) = ws.cols.get_mut(ws.active) else { return };
        c.maximized = false;
        c.width = to.clamp(0.1, 1.0);
        self.settle();
    }

    /// Give the focused window a height of its own: a part of the column's height, or
    /// (`by`) that much more or less than it has now. None: back to sharing the room.
    pub fn set_height(&mut self, to: Option<f64>, by: bool) {
        let Some(ws) = self.active_workspace() else { return };
        let Some(col) = ws.cols.get(ws.active) else { return };
        let Some(&id) = col.wins.get(col.active) else { return };
        match to {
            None => {
                self.heights.remove(&id);
            }
            Some(v) => {
                // What it has now, when it was sharing: an equal part of what the others left.
                let (set, free): (Vec<f64>, Vec<WinId>) = (col.wins.iter().filter_map(|w| self.heights.get(w).copied()).collect(), col.wins.iter().copied().filter(|w| !self.heights.contains_key(w)).collect());
                let now = self.heights.get(&id).copied().unwrap_or_else(|| ((1.0 - set.iter().sum::<f64>()) / free.len().max(1) as f64).max(0.1));
                self.heights.insert(id, if by { now + v } else { v }.clamp(0.1, 1.0));
            }
        }
        self.settle();
    }

    /// How tall each window of a column is, and how far down the first one starts, given
    /// the height there is for windows (the gaps between them already taken off).
    fn column_heights(&self, col: &Column, room: i32) -> (Vec<i32>, i32) {
        let set: Vec<Option<f64>> = col.wins.iter().map(|w| self.heights.get(w).copied()).collect();
        let asked: f64 = set.iter().flatten().sum();
        let free = set.iter().filter(|h| h.is_none()).count();
        // More asked for than there is: everyone gives a little.
        let shrink = if asked > 1.0 || (free > 0 && asked > 0.9) { (if free > 0 { 0.9 } else { 1.0 }) / asked } else { 1.0 };
        let left = room - set.iter().flatten().map(|h| (h * shrink * room as f64).round() as i32).sum::<i32>();
        let mut out: Vec<i32> = set.iter().map(|h| h.map_or(left / free.max(1) as i32, |h| (h * shrink * room as f64).round() as i32).max(1)).collect();
        if free > 0 {
            // The last sharing window takes the pixels the division left over.
            if let Some(i) = set.iter().rposition(|h| h.is_none()) {
                out[i] = (out[i] + room - out.iter().sum::<i32>()).max(1);
            }
            return (out, 0);
        }
        let spare = (room - out.iter().sum::<i32>()).max(0);
        (out, if self.opts.center_short_columns { spare / 2 } else { 0 })
    }

    pub fn toggle_maximized(&mut self) {
        let Some(ws) = self.ws() else { return };
        let Some(c) = ws.cols.get_mut(ws.active) else { return };
        c.maximized = !c.maximized;
        self.settle();
    }

    pub fn center_column(&mut self) {
        let o = self.opts.clone();
        let Some(m) = self.mons.get_mut(self.active_mon) else { return };
        let area = m.area;
        let Some(ws) = m.wss.get_mut(m.active) else { return };
        if ws.cols.is_empty() {
            return;
        }
        let x = ws.col_x(ws.active, &area, &o) as f64;
        let w = ws.col_width(ws.active, &area, &o) as f64;
        ws.view_to = x - ((area.w - 2 * o.gaps_out) as f64 - w) / 2.0;
    }

    pub fn set_fullscreen(&mut self, id: WinId, on: Option<bool>) {
        let Some((mi, wi)) = self.find(id) else { return };
        let ws = &mut self.mons[mi].wss[wi];
        let on = on.unwrap_or(ws.fullscreen != Some(id));
        ws.fullscreen = on.then_some(id);
    }

    // ---- the scratchpad ----

    /// Put the focused window away: out of sight, on no workspace, until asked for.
    pub fn move_to_scratchpad(&mut self) {
        let Some(id) = self.active_window() else { return };
        if let Some(ws) = self.ws() {
            ws.remove(id);
        }
        self.heights.remove(&id);
        self.shown.retain(|w| *w != id);
        self.scratch.push(id);
        self.settle();
    }

    /// The scratchpad windows on show on the workspace in view.
    pub fn scratchpad_in_view(&self) -> Vec<WinId> {
        self.active_workspace().map(|ws| ws.floating.iter().copied().filter(|w| self.shown.contains(w)).collect()).unwrap_or_default()
    }

    /// The scratchpad key: put away what is on show here; else show the next window put
    /// away, floating over this workspace; else bring over one on show somewhere else.
    /// Says which window came into view.
    pub fn toggle_scratchpad(&mut self) -> Option<WinId> {
        let here = self.scratchpad_in_view();
        if !here.is_empty() {
            for id in here {
                if let Some(ws) = self.ws() {
                    ws.remove(id);
                }
                self.shown.retain(|w| *w != id);
                self.scratch.push(id);
            }
            self.settle();
            return None;
        }
        let id = if self.scratch.is_empty() { *self.shown.first()? } else { self.scratch.remove(0) };
        if let Some((mi, wi)) = self.find(id) {
            self.mons[mi].wss[wi].remove(id);
        }
        if !self.shown.contains(&id) {
            self.shown.push(id);
        }
        let ws = self.ws()?;
        ws.floating.push(id);
        ws.float_focus = Some(id);
        self.settle();
        Some(id)
    }

    /// The size a window shown from the scratchpad is asked to be.
    pub fn scratchpad_size(&self, id: WinId) -> Option<(i32, i32)> {
        if !self.shown.contains(&id) {
            return None;
        }
        let m = &self.mons[self.find(id)?.0];
        let by = self.opts.scratchpad_size;
        Some((((m.area.w as f64 * by).round() as i32).max(1), ((m.area.h as f64 * by).round() as i32).max(1)))
    }

    pub fn toggle_floating(&mut self, id: WinId) {
        let Some((mi, wi)) = self.find(id) else { return };
        let width = self.opts.default_width;
        let ws = &mut self.mons[mi].wss[wi];
        if ws.floating.contains(&id) {
            // Into a column, it is a window like any other again.
            self.shown.retain(|w| *w != id);
            let ws = &mut self.mons[mi].wss[wi];
            ws.floating.retain(|w| *w != id);
            ws.float_focus = None;
            ws.insert_column(Column { wins: vec![id], active: 0, width, maximized: false });
        } else {
            ws.remove(id);
            ws.floating.push(id);
            ws.float_focus = Some(id);
        }
        self.settle();
    }

    // ---- workspaces ----

    /// Show a workspace of the focused display (from 0).
    pub fn focus_workspace(&mut self, idx: usize) {
        let Some(m) = self.mons.get_mut(self.active_mon) else { return };
        m.active = idx.min(m.wss.len() - 1);
        self.settle();
    }

    pub fn focus_workspace_by(&mut self, by: i32) {
        let Some(m) = self.mons.get(self.active_mon) else { return };
        let to = (m.active as i32 + by).max(0) as usize;
        self.focus_workspace(to);
    }

    /// Send the focused column (or just the focused window) to another workspace of the same
    /// display; the focus goes with it.
    pub fn move_to_workspace(&mut self, idx: usize, whole_column: bool) {
        let width = self.opts.default_width;
        let Some(m) = self.mons.get_mut(self.active_mon) else { return };
        let to = idx.min(m.wss.len() - 1);
        if to == m.active {
            return;
        }
        let from = m.active;
        let ws = &mut m.wss[from];
        if let Some(f) = ws.float_focus {
            ws.remove(f);
            let t = &mut m.wss[to];
            t.floating.push(f);
            t.float_focus = Some(f);
        } else {
            if ws.cols.is_empty() {
                return;
            }
            let a = ws.active;
            let col = if whole_column || ws.cols[a].wins.len() == 1 {
                if ws.cols[a].wins.iter().any(|w| ws.fullscreen == Some(*w)) {
                    ws.fullscreen = None;
                }
                let c = ws.cols.remove(a);
                ws.active = a.saturating_sub(1);
                c
            } else {
                let id = ws.cols[a].wins[ws.cols[a].active];
                ws.remove(id);
                Column { wins: vec![id], active: 0, width, maximized: false }
            };
            m.wss[to].insert_column(col);
        }
        m.active = to;
        self.settle();
    }

    pub fn move_to_workspace_by(&mut self, by: i32, whole_column: bool) {
        let Some(m) = self.mons.get(self.active_mon) else { return };
        let to = (m.active as i32 + by).max(0) as usize;
        self.move_to_workspace(to, whole_column);
    }

    // ---- across displays ----

    /// The next display in a direction, by the order they were added.
    fn neighbour(&self, by: i32) -> Option<usize> {
        let n = self.mons.iter().filter(|m| !m.orphan).count();
        if n < 2 {
            return None;
        }
        let to = self.active_mon as i32 + by;
        (to >= 0 && (to as usize) < self.mons.len()).then_some(to as usize)
    }

    pub fn focus_monitor(&mut self, by: i32) -> bool {
        match self.neighbour(by) {
            Some(i) => {
                self.active_mon = i;
                true
            }
            None => false,
        }
    }

    pub fn move_column_to_monitor(&mut self, by: i32) {
        let Some(to) = self.neighbour(by) else { return };
        let from = self.active_mon;
        let m = &mut self.mons[from];
        let ws = &mut m.wss[m.active];
        if ws.float_focus.is_some() || ws.cols.is_empty() {
            return;
        }
        let a = ws.active;
        if ws.cols[a].wins.iter().any(|w| ws.fullscreen == Some(*w)) {
            ws.fullscreen = None;
        }
        let col = ws.cols.remove(a);
        ws.active = a.saturating_sub(1);
        let t = &mut self.mons[to];
        let ta = t.active;
        t.wss[ta].insert_column(col);
        self.active_mon = to;
        self.settle();
    }

    // ---- keeping it tidy ----

    /// After any change: an empty workspace last and nowhere else (but the one in view), and
    /// each strip scrolled to its focused column.
    fn settle(&mut self) {
        let o = self.opts.clone();
        for mi in 0..self.mons.len() {
            let mut i = 0;
            while i + 1 < self.mons[mi].wss.len() {
                let m = &mut self.mons[mi];
                if m.wss[i].is_empty() && i != m.active {
                    m.wss.remove(i);
                    if m.active > i {
                        m.active -= 1;
                        m.pos = (m.pos - 1.0).max(0.0);
                    }
                } else {
                    i += 1;
                }
            }
            if self.mons[mi].wss.last().is_none_or(|w| !w.is_empty()) {
                let ws = self.new_ws();
                self.mons[mi].wss.push(ws);
            }
            let m = &mut self.mons[mi];
            m.active = m.active.min(m.wss.len() - 1);
            if !m.orphan {
                for ws in m.wss.iter_mut().filter(|w| w.is_empty() || w.home.is_empty()) {
                    ws.home.clone_from(&m.name);
                }
            }
            let area = m.area;
            for ws in &mut m.wss {
                ws.settle_view(&area, &o);
            }
        }
        if self.active_mon >= self.mons.len() {
            self.active_mon = 0;
        }
        self.note_places();
    }

    /// See where every tiled window stands on its strip now. One that stood elsewhere on the
    /// same strip starts from there and slides over, rather than jumping.
    fn note_places(&mut self) {
        let mut now = HashMap::new();
        let mut tiles = Vec::new();
        for m in &self.mons {
            for ws in &m.wss {
                tiles.clear();
                self.ws_tiles(m, ws, false, &|_| (1, 1), None, &mut tiles);
                now.extend(tiles.iter().filter(|t| !t.floating && !t.fullscreen).map(|t| (t.id, (ws.id, t.rect))));
            }
        }
        self.offsets.retain(|id, _| now.contains_key(id));
        if self.opts.animation_speed > 0.0 {
            for (id, (ws, to)) in &now {
                if let Some((was_ws, from)) = self.places.get(id)
                    && was_ws == ws
                    && from != to
                {
                    let o = self.offsets.entry(*id).or_insert([0.0; 4]);
                    for (o, d) in o.iter_mut().zip([from.x - to.x, from.y - to.y, from.w - to.w, from.h - to.h]) {
                        *o += d as f64;
                    }
                }
            }
        } else {
            self.offsets.clear();
        }
        self.places = now;
    }

    pub fn set_options(&mut self, opts: Options) {
        if self.opts != opts {
            self.opts = opts;
            self.settle();
        }
    }

    /// Move what is sliding (the strips, the workspaces) by `dt` seconds. True while anything
    /// still has somewhere to go.
    pub fn advance(&mut self, dt: f64) -> bool {
        // With animations off everything is where it is going at once.
        let speed = self.opts.animation_speed;
        let k = if speed > 0.0 { 1.0 - (-dt * 16.0 * speed).exp() } else { 1.0 };
        let mut moving = false;
        self.offsets.retain(|_, o| {
            for v in o.iter_mut() {
                *v *= 1.0 - k;
            }
            o.iter().any(|v| v.abs() >= 0.5)
        });
        moving |= !self.offsets.is_empty();
        let mut ease = |v: &mut f64, to: f64| {
            if (*v - to).abs() < 0.5 {
                if *v != to {
                    *v = to;
                }
            } else {
                *v += (to - *v) * k;
                moving = true;
            }
        };
        for m in &mut self.mons {
            // Workspaces slide by whole screens, so ease in screen heights.
            let h = m.size.1.max(1) as f64;
            let mut p = m.pos * h;
            ease(&mut p, m.active as f64 * h);
            m.pos = p / h;
            for ws in &mut m.wss {
                ease(&mut ws.view, ws.view_to);
            }
        }
        moving
    }

    /// Jump to where everything is heading.
    #[cfg(test)]
    pub fn finish_animations(&mut self) {
        self.offsets.clear();
        for m in &mut self.mons {
            m.pos = m.active as f64;
            for ws in &mut m.wss {
                ws.view = ws.view_to;
            }
        }
    }

    // ---- where things are ----

    /// `view`: how far the strip is scrolled, as drawn now (windows still sliding are where
    /// they have got to). None: where every window stands on the strip itself, at rest.
    fn ws_tiles(&self, m: &Monitor, ws: &Workspace, focused: bool, float_size: &dyn Fn(WinId) -> (i32, i32), view: Option<f64>, out: &mut Vec<Tile>) {
        let o = &self.opts;
        let area = &m.area;
        let sliding = view.is_some();
        let view = view.unwrap_or(0.0);
        let b = o.border;
        for (ci, col) in ws.cols.iter().enumerate() {
            let w = ws.col_width(ci, area, o);
            let x = area.x + o.gaps_out + ws.col_x(ci, area, o) - view.round() as i32;
            let n = col.wins.len() as i32;
            let room = area.h - 2 * o.gaps_out - o.gaps * (n - 1);
            let (heights, down) = self.column_heights(col, room);
            let mut next = area.y + o.gaps_out + down;
            for (ri, id) in col.wins.iter().enumerate() {
                let (y, h) = (next, heights[ri]);
                next += h + o.gaps;
                let active = focused && ws.float_focus.is_none() && ci == ws.active && ri == col.active;
                let mut rect = Rect::new(x + b, y + b, (w - 2 * b).max(1), (h - 2 * b).max(1));
                let off = self.offsets.get(id).filter(|_| sliding);
                if let Some(o) = off {
                    rect = Rect::new(rect.x + o[0].round() as i32, rect.y + o[1].round() as i32, (rect.w + o[2].round() as i32).max(1), (rect.h + o[3].round() as i32).max(1));
                }
                if ws.fullscreen != Some(*id) {
                    out.push(Tile { id: *id, rect, border: b > 0, active, floating: false, fullscreen: false, moving: off.is_some() });
                }
            }
        }
        for id in &ws.floating {
            let (w, h) = float_size(*id);
            let (w, h) = (w.clamp(1, area.w.max(1)), h.clamp(1, area.h.max(1)));
            let rect = Rect::new(area.x + (area.w - w) / 2, area.y + (area.h - h) / 2, w, h);
            if ws.fullscreen != Some(*id) {
                out.push(Tile { id: *id, rect, border: b > 0, active: focused && ws.float_focus == Some(*id), floating: true, fullscreen: false, moving: false });
            }
        }
        if let Some(id) = ws.fullscreen {
            out.push(Tile { id, rect: Rect::new(0, 0, m.size.0, m.size.1), border: false, active: focused, floating: false, fullscreen: true, moving: false });
        }
    }

    /// The windows to draw on a display now, bottom first, with the strip's scroll and the
    /// slide between workspaces applied. Windows wholly off screen are left out.
    pub fn tiles(&self, mon: usize, float_size: &dyn Fn(WinId) -> (i32, i32)) -> Vec<Tile> {
        let Some(m) = self.mons.get(mon) else { return Vec::new() };
        let mut out = Vec::new();
        let screen = Rect::new(0, 0, m.size.0, m.size.1);
        for (wi, ws) in m.wss.iter().enumerate() {
            let off = wi as f64 - m.pos;
            if off.abs() >= 1.0 {
                continue;
            }
            let dy = (off * m.size.1 as f64).round() as i32;
            let start = out.len();
            self.ws_tiles(m, ws, mon == self.active_mon && wi == m.active, float_size, Some(ws.view), &mut out);
            for t in &mut out[start..] {
                t.rect.y += dy;
            }
        }
        out.retain(|t| t.rect.intersects(&screen));
        out
    }

    /// The size every window of a display should be, whether in view or not.
    pub fn sizes(&self, mon: usize, float_size: &dyn Fn(WinId) -> (i32, i32)) -> Vec<Tile> {
        let Some(m) = self.mons.get(mon) else { return Vec::new() };
        let mut out = Vec::new();
        for (wi, ws) in m.wss.iter().enumerate() {
            let start = out.len();
            self.ws_tiles(m, ws, mon == self.active_mon && wi == m.active, float_size, None, &mut out);
            // Not that it matters to a size, but as it was: where the strip is heading.
            for t in &mut out[start..] {
                if !t.floating && !t.fullscreen {
                    t.rect.x -= ws.view_to.round() as i32;
                }
            }
        }
        out
    }

    /// The size a window about to open would get, so its first frame is already the right one.
    pub fn new_window_size(&self, on: Option<&str>) -> Option<(i32, i32)> {
        let m = on.and_then(|n| self.monitor(n)).and_then(|i| self.mons.get(i)).or(self.mons.get(self.active_mon))?;
        let o = &self.opts;
        let full = m.area.w - 2 * o.gaps_out;
        let w = (((full + o.gaps) as f64) * o.default_width).round() as i32 - o.gaps;
        Some(((w - 2 * o.border).max(1), (m.area.h - 2 * o.gaps_out - 2 * o.border).max(1)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Layout {
        let mut l = Layout::new(Options { gaps: 10, gaps_out: 10, border: 2, ..Options::default() });
        l.add_monitor("A", (1000, 600));
        l
    }

    fn fs(_: WinId) -> (i32, i32) {
        (300, 200)
    }

    #[test]
    fn a_shorter_window_sits_in_the_middle() {
        let mut l = layout();
        l.add_window(1, false, None);
        l.finish_animations();
        let rect = |l: &Layout, id| l.sizes(0, &fs).iter().find(|t| t.id == id).map(|t| (t.rect.y, t.rect.h)).unwrap();
        // 600 high, 10 at each edge, a border of 2: 580 for the column.
        assert_eq!(rect(&l, 1), (12, 576));
        l.set_height(Some(0.5), false);
        assert_eq!(rect(&l, 1), (10 + 145 + 2, 290 - 4));
        l.set_height(Some(0.1), true);
        assert_eq!(rect(&l, 1), (10 + 116 + 2, 348 - 4));
        // At the top instead, when the settings say so.
        l.set_options(Options { center_short_columns: false, ..l.opts.clone() });
        assert_eq!(rect(&l, 1), (12, 348 - 4));
        l.set_options(Options { center_short_columns: true, ..l.opts.clone() });

        // A second window in the column takes what is left: nothing to centre.
        l.add_window(2, false, None);
        l.consume_or_expel(Dir::Left);
        let (a, b) = (rect(&l, 1), rect(&l, 2));
        assert_eq!((a.0, b.0 + b.1), (12, 588));
        assert_eq!(a.1 + 4, (570.0_f64 * 0.6).round() as i32);
        // Both with heights of their own: together in the middle.
        l.set_height(Some(0.2), false);
        let (a, b) = (rect(&l, 1), rect(&l, 2));
        assert_eq!(a.0 - 12, 588 - (b.0 + b.1));
        // And back to filling the column.
        l.set_height(None, false);
        l.focus(Dir::Up);
        l.set_height(None, false);
        let (a, b) = (rect(&l, 1), rect(&l, 2));
        assert_eq!((a.0, b.0 + b.1, a.1), (12, 588, 281));
    }

    #[test]
    fn the_scratchpad_puts_windows_away_and_brings_them_back() {
        let mut l = layout();
        l.add_window(1, false, None);
        l.add_window(2, false, None);
        l.move_to_scratchpad();
        assert_eq!((l.find(2), l.active_window(), l.scratch.clone()), (None, Some(1), vec![2]));
        // Shown: floating, focused, at the scratchpad's size.
        assert_eq!(l.toggle_scratchpad(), Some(2));
        assert_eq!((l.is_floating(2), l.active_window(), l.scratchpad_size(2)), (true, Some(2), Some((700, 420))));
        assert_eq!(l.scratchpad_size(1), None);
        // And away again, with the focus back where it was.
        assert_eq!(l.toggle_scratchpad(), None);
        assert_eq!((l.find(2), l.active_window()), (None, Some(1)));
        // On show on one workspace, asked for on another: it comes over.
        l.toggle_scratchpad();
        l.focus_workspace_by(1);
        assert_eq!(l.toggle_scratchpad(), Some(2));
        assert_eq!(l.find(2), Some((0, 1)));
        // Two put away take turns.
        l.toggle_scratchpad();
        l.focus_workspace_by(-1);
        l.move_to_scratchpad();
        assert_eq!(l.scratch, vec![2, 1]);
        assert_eq!(l.toggle_scratchpad(), Some(2));
        l.toggle_scratchpad();
        assert_eq!(l.toggle_scratchpad(), Some(1));
        // Put into a column, it is an ordinary window again; closed, it is forgotten.
        l.toggle_floating(1);
        assert_eq!((l.toggle_scratchpad(), l.is_floating(1)), (Some(2), false));
        l.remove_window(2);
        assert_eq!(l.toggle_scratchpad(), None);
        assert!(l.scratch.is_empty());
    }

    #[test]
    fn windows_slide_to_their_places() {
        let mut l = layout();
        l.add_window(1, false, None);
        l.add_window(2, false, None);
        l.finish_animations();
        let x = |l: &Layout, id| l.tiles(0, &fs).iter().find(|t| t.id == id).map(|t| (t.rect.x, t.moving)).unwrap();
        let (from, to) = (x(&l, 2).0, x(&l, 1).0);
        l.move_column(Dir::Left);
        // It starts where it stood, is somewhere between a moment later, and arrives.
        assert_eq!(x(&l, 2), (from, true));
        assert!(l.advance(0.05));
        let (mid, moving) = x(&l, 2);
        assert!(moving && to < mid && mid < from, "{to} < {mid} < {from}");
        // The size windows are told is the one they are heading for, from the start.
        assert_eq!(l.sizes(0, &fs).iter().find(|t| t.id == 2).unwrap().rect.x, to);
        for _ in 0..100 {
            l.advance(0.05);
        }
        assert_eq!(x(&l, 2), (to, false));

        // With animations off nothing slides.
        l.set_options(Options { animation_speed: 0.0, ..l.opts.clone() });
        l.move_column(Dir::Right);
        assert_eq!(x(&l, 2), (from, false));
    }

    #[test]
    fn two_halves_fill_the_screen() {
        let mut l = layout();
        l.add_window(1, false, None);
        l.add_window(2, false, None);
        l.finish_animations();
        let t = l.tiles(0, &fs);
        assert_eq!(t.len(), 2);
        // Tile = window + border: 10 | 485 | 10 | 485 | 10.
        assert_eq!(t[0].rect, Rect::new(12, 12, 481, 576));
        assert_eq!(t[1].rect, Rect::new(507, 12, 481, 576));
        assert!(t[1].active && !t[0].active);
        assert_eq!(l.new_window_size(None), Some((481, 576)));
    }

    #[test]
    fn the_strip_scrolls_to_the_focused_column() {
        let mut l = layout();
        for id in 1..=3 {
            l.add_window(id, false, None);
        }
        l.finish_animations();
        // The third column is in view; the first has scrolled off.
        let ids: Vec<WinId> = l.tiles(0, &fs).iter().map(|t| t.id).collect();
        assert_eq!(ids, vec![2, 3]);
        l.focus(Dir::Left);
        l.focus(Dir::Left);
        assert_eq!(l.active_window(), Some(1));
        // Sliding there: part way first, then arrived.
        assert!(l.advance(0.016));
        l.finish_animations();
        let ids: Vec<WinId> = l.tiles(0, &fs).iter().map(|t| t.id).collect();
        assert_eq!(ids, vec![1, 2]);
        assert!(!l.advance(0.016));
    }

    #[test]
    fn closing_moves_the_focus_left() {
        let mut l = layout();
        for id in 1..=3 {
            l.add_window(id, false, None);
        }
        l.remove_window(3);
        assert_eq!(l.active_window(), Some(2));
        l.focus(Dir::Left);
        l.remove_window(1);
        assert_eq!(l.active_window(), Some(2));
        l.remove_window(2);
        assert_eq!(l.active_window(), None);
    }

    #[test]
    fn consume_and_expel() {
        let mut l = layout();
        l.add_window(1, false, None);
        l.add_window(2, false, None);
        l.consume_or_expel(Dir::Left);
        assert_eq!(l.position(2), Some((1, 2)));
        assert_eq!(l.active_window(), Some(2));
        l.finish_animations();
        let t = l.tiles(0, &fs);
        assert_eq!(t[0].rect.h + t[1].rect.h + 4 * 2 + 10, 600 - 20);
        l.focus(Dir::Up);
        assert_eq!(l.active_window(), Some(1));
        l.consume_or_expel(Dir::Right);
        assert_eq!(l.position(1), Some((2, 1)));
        assert_eq!(l.position(2), Some((1, 1)));
    }

    #[test]
    fn workspaces_come_and_go() {
        let mut l = layout();
        assert_eq!(l.mons[0].wss.len(), 1);
        l.add_window(1, false, None);
        assert_eq!(l.mons[0].wss.len(), 2);
        l.add_window(2, false, None);
        l.move_to_workspace_by(1, true);
        assert_eq!(l.mons[0].active, 1);
        assert_eq!(l.mons[0].wss.len(), 3);
        assert_eq!(l.active_window(), Some(2));
        l.focus_workspace(0);
        assert_eq!(l.active_window(), Some(1));
        // The first workspace empties while we are elsewhere: it goes.
        l.focus_workspace(1);
        l.remove_window(1);
        assert_eq!(l.mons[0].wss.len(), 2);
        assert_eq!(l.mons[0].active, 0);
        assert_eq!(l.active_window(), Some(2));
    }

    #[test]
    fn widths_and_fullscreen() {
        let mut l = layout();
        l.add_window(1, false, None);
        l.switch_preset_width();
        assert!((l.active_workspace().unwrap().cols[0].width - 2.0 / 3.0).abs() < 1e-9);
        l.switch_preset_width();
        assert!((l.active_workspace().unwrap().cols[0].width - 1.0 / 3.0).abs() < 1e-9);
        l.toggle_maximized();
        l.finish_animations();
        assert_eq!(l.tiles(0, &fs)[0].rect.w, 1000 - 20 - 4);
        l.set_fullscreen(1, None);
        let t = l.tiles(0, &fs);
        assert_eq!((t[0].rect, t[0].border), (Rect::new(0, 0, 1000, 600), false));
    }

    #[test]
    fn floating_windows_sit_in_the_middle() {
        let mut l = layout();
        l.add_window(1, false, None);
        l.add_window(2, true, None);
        assert_eq!(l.active_window(), Some(2));
        l.finish_animations();
        let t = l.tiles(0, &fs);
        assert_eq!(t[1].rect, Rect::new(350, 200, 300, 200));
        l.switch_floating_focus();
        assert_eq!(l.active_window(), Some(1));
        l.toggle_floating(2);
        assert_eq!(l.position(2), Some((2, 1)));
    }

    #[test]
    fn a_display_leaving_hands_over_its_workspaces() {
        let mut l = layout();
        l.add_monitor("B", (800, 600));
        l.add_window(1, false, Some("B"));
        assert_eq!(l.find(1), Some((1, 0)));
        l.remove_monitor("B");
        assert_eq!(l.find(1), Some((0, 0)));
        assert_eq!(l.mons.len(), 1);
        // Back again, it gets its own back, and only its own.
        l.focus_workspace_by(1);
        l.add_window(2, false, None);
        l.add_monitor("B", (800, 600));
        assert_eq!((l.find(1), l.find(2)), (Some((1, 0)), Some((0, 0))));
        assert!(l.mons.iter().all(|m| m.wss.last().is_some_and(|w| w.is_empty())));
        // A window moved to another display by hand stays there.
        l.focus_window(1);
        l.move_column_to_monitor(-1);
        l.remove_monitor("B");
        l.add_monitor("B", (800, 600));
        assert_eq!(l.find(1).map(|p| p.0), Some(0));

        // The last display going keeps everything for the next one, and still gives
        // back what belongs to one that returns.
        l.add_window(3, false, Some("B"));
        l.remove_monitor("B");
        l.remove_monitor("A");
        l.add_monitor("C", (800, 600));
        assert_eq!((l.find(1).map(|p| p.0), l.find(3).map(|p| p.0), l.mons[0].name.as_str()), (Some(0), Some(0), "C"));
        l.add_monitor("B", (800, 600));
        assert_eq!((l.find(1).map(|p| p.0), l.find(3).map(|p| p.0)), (Some(0), Some(1)));
    }
}
