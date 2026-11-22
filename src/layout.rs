//! The scrolling layout: windows sit in columns on a strip that runs off both sides of the
//! screen, and each display has its own stack of workspaces, with an empty one always last.
//!
//! Nothing here knows about Wayland. Windows are numbers, rectangles are in a display's own
//! logical pixels, and the compositor asks for `tiles()` each frame. That keeps every rule of
//! the layout testable without a screen.

use std::collections::HashMap;

pub use crate::config::WorkspaceRef;

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

pub use softcore_settings::{Dir, Options, Scroll};

/// The same rectangle with across and down changed over: a strip that runs down the screen
/// is laid out as one that runs across it, and turned.
fn turned(r: Rect) -> Rect {
    Rect::new(r.y, r.x, r.h, r.w)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub wins: Vec<WinId>,
    pub active: usize,
    /// Part of the screen's width.
    pub width: f64,
    /// As wide as the screen allows, until toggled back.
    pub maximized: bool,
    /// Its windows are tabs: each has the whole column, and only the active one shows.
    pub tabbed: bool,
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
    /// Which way it scrolls, when it was told: it keeps that wherever it goes. None: as
    /// the display it is on does.
    pub scroll: Option<Scroll>,
    /// A workspace with a name is one the settings ask for: it stays when it is empty.
    pub name: Option<String>,
    /// Which way its own line in the settings says it scrolls.
    pub named_scroll: Option<Scroll>,
}

impl Workspace {
    fn new(id: u64) -> Workspace {
        Workspace { id, cols: Vec::new(), active: 0, view: 0.0, view_to: 0.0, floating: Vec::new(), float_focus: None, fullscreen: None, home: String::new(), scroll: None, name: None, named_scroll: None }
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
    /// Which way its windows scroll, when its own line in the settings says. None: as the
    /// `scroll` setting says.
    pub scroll: Option<Scroll>,
}

impl Monitor {
    /// Its strips run down the screen: rows one under the other, scrolling up and down.
    pub fn vertical(&self, o: &Options) -> bool {
        match self.scroll.unwrap_or(o.scroll) {
            Scroll::Horizontal => false,
            Scroll::Vertical => true,
            Scroll::Auto => self.size.1 > self.size.0,
        }
    }

    /// Whether one of its workspaces scrolls up and down: as the workspace was told, else
    /// as the display does.
    pub fn ws_vertical(&self, ws: &Workspace, o: &Options) -> bool {
        match ws.scroll.or(ws.named_scroll) {
            Some(Scroll::Horizontal) => false,
            Some(Scroll::Vertical) => true,
            Some(Scroll::Auto) => self.size.1 > self.size.0,
            None => self.vertical(o),
        }
    }

    /// The room for windows as a strip sees it: along the strip is always across.
    fn strip_area(&self, vertical: bool) -> Rect {
        if vertical { turned(self.area) } else { self.area }
    }
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
    /// Changing size on the way.
    pub resizing: bool,
    /// Drawn this much smaller than it is (in the overview). 1: as it is.
    pub zoom: f64,
    /// In a tabbed column: its piece of the strip of tabs.
    pub tab: Option<Rect>,
    /// A tab that isn't the one showing: it has its size and its piece of the strip, and
    /// isn't drawn.
    pub hidden: bool,
}

/// Where a floating window was put and the size it was given, when it was: until then it
/// sits in the middle at the size it chose.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Float {
    /// Its corner, from the corner of the part of the display panels leave free.
    pub pos: Option<(i32, i32)>,
    pub size: Option<(i32, i32)>,
    /// While it is resized by its left or its top edge: the right, the bottom edge is what
    /// stays put, whatever size the window itself has got to so far.
    pub hold: (bool, bool),
}

/// Where a window let go of with the pointer goes: by a window that is already there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spot {
    /// A column of its own, before that window's column, or after it.
    Before(WinId),
    After(WinId),
    /// Into that window's column, above it or below.
    Above(WinId),
    Below(WinId),
    /// The last column of the workspace in view on a display (by its place among them).
    Last(usize),
}

/// Between two pieces of a strip of tabs.
const TAB_GAP: i32 = 4;

/// How much of a floating window always stays on its display, to take hold of.
const HOLD: i32 = 48;

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
    floats: HashMap<WinId, Float>,
    pub scratch: Vec<WinId>,
    /// Windows out of the scratchpad and on show: the scratchpad key puts these away again.
    shown: Vec<WinId>,
    /// The scratchpad each of those windows belongs to, when it is one with a name.
    scratch_names: HashMap<WinId, String>,
    /// How much of the screen a named scratchpad's window takes, where its line says.
    scratch_sizes: HashMap<String, f64>,
    /// Fingers on the touchpad, moving what is in view.
    swipe: Option<Swipe>,
}

/// A swipe on the touchpad, from fingers down to fingers up.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Swipe {
    /// How far the fingers have gone, before it is clear which way they are going.
    moved: (f64, f64),
    /// Along the strip of windows, or across it through the workspaces. None: not yet clear.
    along: Option<bool>,
}

/// How a window opens.
#[derive(Clone, Debug, PartialEq)]
pub struct Open<'a> {
    pub floating: bool,
    pub output: Option<&'a str>,
    pub workspace: Option<&'a WorkspaceRef>,
    /// Part of the screen's width.
    pub width: Option<f64>,
    pub maximized: bool,
    /// False: it opens where it is told and the focus stays where it was.
    pub focus: bool,
}

impl Default for Open<'_> {
    fn default() -> Self {
        Open { floating: false, output: None, workspace: None, width: None, maximized: false, focus: true }
    }
}

/// What the scratchpad key did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scratch {
    /// What was on show is put away.
    Hidden,
    Shown(WinId),
    /// There was nothing to show.
    Empty,
}

impl Layout {
    pub fn new(opts: Options) -> Layout {
        Layout { mons: Vec::new(), active_mon: 0, opts, next_ws: 1, places: HashMap::new(), offsets: HashMap::new(), heights: HashMap::new(), floats: HashMap::new(), scratch: Vec::new(), shown: Vec::new(), scratch_names: HashMap::new(), scratch_sizes: HashMap::new(), swipe: None }
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
            let mut new = Monitor { name: name.to_string(), wss: vec![ws], active: 0, pos: 0.0, size, area, orphan: false, scroll: None };
            // Back after being away: the workspaces that went to other displays come home.
            for m in &mut self.mons {
                let mut i = 0;
                while i < m.wss.len() {
                    if m.wss[i].home == name && (!m.wss[i].is_empty() || m.wss[i].name.is_some()) {
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
        for (k, ws) in gone.wss.into_iter().filter(|w| !w.is_empty() || w.name.is_some()).enumerate() {
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

    /// What a display's own line in the settings says about which way it scrolls.
    pub fn set_scroll(&mut self, name: &str, scroll: Option<Scroll>) {
        let Some(m) = self.mons.iter_mut().find(|m| m.name == name) else { return };
        if m.scroll != scroll {
            m.scroll = scroll;
            self.settle();
        }
    }

    /// Whether the workspace in view on the focused display scrolls up and down.
    pub fn vertical(&self) -> bool {
        self.active_monitor().is_some_and(|m| m.wss.get(m.active).is_some_and(|ws| m.ws_vertical(ws, &self.opts)))
    }

    /// Tell the workspace in view which way to scroll. `Some(None)`: as its display does.
    /// None: the other way from now.
    pub fn set_workspace_scroll(&mut self, to: Option<Option<Scroll>>) {
        let now = self.vertical();
        let Some(ws) = self.ws() else { return };
        ws.scroll = to.unwrap_or(Some(if now { Scroll::Horizontal } else { Scroll::Vertical }));
        self.settle();
    }

    /// A direction on the screen as a direction on the strip, which is laid out as if it
    /// ran across: on a display that scrolls up and down, down is the next row.
    fn on_strip(&self, dir: Dir) -> Dir {
        if !self.vertical() {
            return dir;
        }
        match dir {
            Dir::Left => Dir::Up,
            Dir::Right => Dir::Down,
            Dir::Up => Dir::Left,
            Dir::Down => Dir::Right,
        }
    }

    /// The way to the next column (or row), or the one before, on the screen.
    pub fn along(&self, forward: bool) -> Dir {
        match (self.vertical(), forward) {
            (false, true) => Dir::Right,
            (false, false) => Dir::Left,
            (true, true) => Dir::Down,
            (true, false) => Dir::Up,
        }
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
    #[cfg(test)]
    pub fn add_window(&mut self, id: WinId, floating: bool, on: Option<&str>) {
        self.open_window(id, Open { floating, output: on, ..Default::default() });
    }

    /// A new window, where and how its rules say.
    pub fn open_window(&mut self, id: WinId, open: Open) {
        // The workspace it names, else the one in view on the display it names, else the
        // one with the focus.
        let (mi, wi) = match open.workspace.and_then(|w| self.workspace_at(w)) {
            Some(at) => at,
            None => {
                let mi = open.output.and_then(|n| self.monitor(n)).unwrap_or(self.active_mon);
                let Some(m) = self.mons.get(mi) else { return };
                (mi, m.active)
            }
        };
        let width = open.width.unwrap_or(self.opts.default_width);
        let ws = &mut self.mons[mi].wss[wi];
        let was = (ws.active, ws.float_focus, ws.cols.is_empty() && ws.floating.is_empty());
        if open.focus {
            // It is not to open out of sight behind a window that has the whole screen.
            ws.fullscreen = None;
        }
        if open.floating {
            ws.floating.push(id);
            ws.float_focus = Some(id);
        } else {
            ws.insert_column(Column { wins: vec![id], active: 0, width, maximized: open.maximized, tabbed: false });
        }
        if open.focus {
            self.active_mon = mi;
            self.mons[mi].active = wi;
        } else if !was.2 {
            // The focus stays with what had it. The new column went in after it.
            ws.active = was.0;
            ws.float_focus = was.1;
            if open.floating && was.1.is_some() {
                // Under the floating window that has the focus, not over it.
                let n = ws.floating.len();
                ws.floating.swap(n - 1, n - 2);
            }
        }
        self.settle();
    }

    // ---- workspaces with names ----

    /// The workspaces the settings name, in their order: each is made if it isn't there,
    /// on the display it asks for, and kept when empty. One whose line has gone is a
    /// workspace like any other again.
    pub fn set_named_workspaces(&mut self, wanted: &[(String, Option<String>, Option<Scroll>)]) {
        if self.mons.is_empty() {
            return;
        }
        for ws in self.mons.iter_mut().flat_map(|m| m.wss.iter_mut()) {
            if ws.name.as_ref().is_some_and(|n| !wanted.iter().any(|w| w.0 == *n)) {
                ws.name = None;
                ws.named_scroll = None;
            }
        }
        for (name, output, scroll) in wanted {
            let to = output.as_deref().and_then(|o| self.monitor(o));
            let at = self.workspace_at(&WorkspaceRef::Name(name.clone()));
            let mut ws = match at {
                // Where it should be already.
                Some((mi, wi)) if to.is_none_or(|t| t == mi) => {
                    let ws = &mut self.mons[mi].wss[wi];
                    ws.named_scroll = *scroll;
                    if let Some(o) = output {
                        ws.home.clone_from(o);
                    }
                    continue;
                }
                Some((mi, wi)) => {
                    let m = &mut self.mons[mi];
                    let ws = m.wss.remove(wi);
                    if wi < m.active {
                        m.active -= 1;
                        m.pos = (m.pos - 1.0).max(0.0);
                    }
                    ws
                }
                None => {
                    let mut ws = self.new_ws();
                    ws.name = Some(name.clone());
                    ws
                }
            };
            ws.named_scroll = *scroll;
            ws.home = output.clone().unwrap_or_default();
            // After the named ones already there, before the rest.
            let m = &mut self.mons[to.unwrap_or(0)];
            let i = m.wss.iter().take_while(|w| w.name.is_some()).count();
            let fresh = m.wss.len() == 1 && m.wss[0].is_empty() && m.wss[0].name.is_none();
            m.wss.insert(i, ws);
            // A display that had nothing yet starts on its first named workspace.
            if i <= m.active && !fresh {
                m.active += 1;
                m.pos += 1.0;
            }
        }
        self.settle();
    }

    /// The display and place in its stack of a workspace: a number counts on the focused
    /// display, a name is looked for on all of them.
    pub fn workspace_at(&self, r: &WorkspaceRef) -> Option<(usize, usize)> {
        match r {
            WorkspaceRef::Index(i) => {
                let m = self.mons.get(self.active_mon)?;
                Some((self.active_mon, (*i).min(m.wss.len().checked_sub(1)?)))
            }
            WorkspaceRef::Name(n) => self.mons.iter().enumerate().find_map(|(mi, m)| m.wss.iter().position(|w| w.name.as_deref() == Some(n.as_str())).map(|wi| (mi, wi))),
        }
    }

    /// Show a workspace, on whichever display it is.
    pub fn focus_workspace_ref(&mut self, r: &WorkspaceRef) {
        let Some((mi, wi)) = self.workspace_at(r) else { return };
        self.active_mon = mi;
        self.mons[mi].active = wi;
        self.settle();
    }

    /// Send the focused column (or just the focused window) to a workspace, on whichever
    /// display it is; the focus goes with it.
    pub fn move_to_workspace_ref(&mut self, r: &WorkspaceRef, whole_column: bool) {
        let Some((tm, to)) = self.workspace_at(r) else { return };
        let width = self.opts.default_width;
        let from_mon = self.active_mon;
        let Some(m) = self.mons.get_mut(from_mon) else { return };
        if (tm, to) == (from_mon, m.active) {
            return;
        }
        // The target's id, to find it again if taking the window away shifts the stack.
        let target = self.mons[tm].wss[to].id;
        let m = &mut self.mons[from_mon];
        let ws = &mut m.wss[m.active];
        let moved = if let Some(f) = ws.float_focus {
            ws.remove(f);
            Err(f)
        } else {
            if ws.cols.is_empty() {
                return;
            }
            let a = ws.active;
            Ok(if whole_column || ws.cols[a].wins.len() == 1 {
                if ws.cols[a].wins.iter().any(|w| ws.fullscreen == Some(*w)) {
                    ws.fullscreen = None;
                }
                let c = ws.cols.remove(a);
                ws.active = a.saturating_sub(1);
                c
            } else {
                let id = ws.cols[a].wins[ws.cols[a].active];
                ws.remove(id);
                Column { wins: vec![id], active: 0, width, maximized: false, tabbed: false }
            })
        };
        let Some(to) = self.mons[tm].wss.iter().position(|w| w.id == target) else { return };
        let t = &mut self.mons[tm].wss[to];
        match moved {
            Ok(col) => t.insert_column(col),
            Err(f) => {
                t.floating.push(f);
                t.float_focus = Some(f);
            }
        }
        self.active_mon = tm;
        self.mons[tm].active = to;
        self.settle();
    }

    pub fn remove_window(&mut self, id: WinId) {
        self.scratch.retain(|w| *w != id);
        self.shown.retain(|w| *w != id);
        self.scratch_names.remove(&id);
        self.floats.remove(&id);
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
        let dir = self.on_strip(dir);
        let Some(ws) = self.ws() else { return };
        ws.float_focus = None;
        // Going to another window leaves full screen: the focus is never out of sight.
        ws.fullscreen = None;
        if ws.cols.is_empty() {
            self.settle();
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

    /// As `focus`, but where there is no window further that way in the column (the row),
    /// on to the workspace that way.
    pub fn focus_or_workspace(&mut self, dir: Dir) {
        let across = match self.on_strip(dir) {
            Dir::Up => Some(-1),
            Dir::Down => Some(1),
            _ => None,
        };
        let at_end = |ws: &Workspace, by: i32| ws.float_focus.is_some() || ws.cols.get(ws.active).is_none_or(|c| if by < 0 { c.active == 0 } else { c.active + 1 >= c.wins.len() });
        match (across, self.active_workspace()) {
            (Some(by), Some(ws)) if at_end(ws, by) => self.focus_workspace_by(by),
            _ => self.focus(dir),
        }
    }

    pub fn focus_column_edge(&mut self, last: bool) {
        let Some(ws) = self.ws() else { return };
        ws.float_focus = None;
        ws.fullscreen = None;
        ws.active = if last { ws.cols.len().saturating_sub(1) } else { 0 };
        self.settle();
    }

    /// Between the floating windows and the columns under them.
    pub fn switch_floating_focus(&mut self) {
        let Some(ws) = self.ws() else { return };
        ws.fullscreen = None;
        ws.float_focus = match ws.float_focus {
            Some(_) if !ws.cols.is_empty() => None,
            Some(f) => Some(f),
            None => ws.floating.last().copied(),
        };
    }

    // ---- moving windows ----

    pub fn move_column(&mut self, dir: Dir) {
        let dir = self.on_strip(dir);
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
            ws.cols.insert(at, Column { wins: vec![id], active: 0, width, maximized: false, tabbed: false });
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

    /// The focused column's windows as tabs, or all showing again.
    pub fn toggle_tabbed(&mut self) {
        let Some(ws) = self.ws() else { return };
        let Some(c) = ws.cols.get_mut(ws.active) else { return };
        c.tabbed = !c.tabbed;
        self.settle();
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
        let Some(ws) = m.wss.get(m.active) else { return };
        let area = m.strip_area(m.ws_vertical(ws, &o));
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
        if on {
            ws.fullscreen = Some(id);
        } else if ws.fullscreen == Some(id) {
            ws.fullscreen = None;
        }
        self.settle();
    }

    /// Take a tiled window from where it is and put it down at a spot; it has the focus there.
    pub fn drop_window(&mut self, id: WinId, spot: Spot) {
        let Some((mi, wi)) = self.find(id) else { return };
        if matches!(spot, Spot::Before(a) | Spot::After(a) | Spot::Above(a) | Spot::Below(a) if a == id) || self.mons[mi].wss[wi].floating.contains(&id) {
            return;
        }
        // A window that had a column to itself keeps that column's width.
        let from = &self.mons[mi].wss[wi];
        let alone = from.cols.iter().find(|c| c.wins == [id]).map(|c| (c.width, c.maximized));
        let (width, maximized) = alone.unwrap_or((self.opts.default_width, false));
        let col = Column { wins: vec![id], active: 0, width, maximized, tabbed: false };
        self.mons[mi].wss[wi].remove(id);
        self.heights.remove(&id);
        let (tm, tw) = match spot {
            Spot::Last(m) if m < self.mons.len() => (m, self.mons[m].active),
            Spot::Before(a) | Spot::After(a) | Spot::Above(a) | Spot::Below(a) if self.find(a).is_some() => self.find(a).unwrap(),
            // Nowhere to go after all: back as a column where it was.
            _ => (mi, wi.min(self.mons[mi].wss.len() - 1)),
        };
        let ws = &mut self.mons[tm].wss[tw];
        let anchor = |ws: &Workspace, a: WinId| ws.cols.iter().enumerate().find_map(|(c, col)| col.wins.iter().position(|w| *w == a).map(|r| (c, r)));
        match spot {
            Spot::Before(a) | Spot::After(a) if anchor(ws, a).is_some() => {
                let at = anchor(ws, a).unwrap().0 + usize::from(matches!(spot, Spot::After(_)));
                ws.cols.insert(at, col);
                ws.active = at;
            }
            Spot::Above(a) | Spot::Below(a) if anchor(ws, a).is_some() => {
                let (c, r) = anchor(ws, a).unwrap();
                let at = r + usize::from(matches!(spot, Spot::Below(_)));
                ws.cols[c].wins.insert(at, id);
                ws.cols[c].active = at;
                ws.active = c;
            }
            _ => {
                ws.cols.push(col);
                ws.active = ws.cols.len() - 1;
            }
        }
        ws.float_focus = None;
        ws.fullscreen = None;
        self.active_mon = tm;
        self.mons[tm].active = tw;
        self.settle();
    }

    /// Send a window to another display, a column of its own on the workspace in view there.
    pub fn move_window_to_monitor(&mut self, id: WinId, name: &str) {
        let (Some((mi, wi)), Some(to)) = (self.find(id), self.monitor(name)) else { return };
        if mi == to {
            return;
        }
        let floating = self.mons[mi].wss[wi].floating.contains(&id);
        let width = self.opts.default_width;
        self.mons[mi].wss[wi].remove(id);
        self.heights.remove(&id);
        let t = &mut self.mons[to];
        let ws = &mut t.wss[t.active];
        if floating {
            ws.floating.push(id);
            ws.float_focus = Some(id);
        } else {
            ws.insert_column(Column { wins: vec![id], active: 0, width, maximized: false, tabbed: false });
        }
        self.active_mon = to;
        self.settle();
    }

    // ---- the scratchpad ----

    /// Put the focused window away: out of sight, on no workspace, until asked for. With a
    /// name it goes into that scratchpad, which has a key of its own.
    pub fn move_to_scratchpad(&mut self, name: Option<&str>) {
        let Some(id) = self.active_window() else { return };
        if let Some(ws) = self.ws() {
            ws.remove(id);
        }
        self.put_away(id, name);
        self.settle();
    }

    /// Put a window that is on no workspace into a scratchpad: one that opens there.
    pub fn put_away(&mut self, id: WinId, name: Option<&str>) {
        self.heights.remove(&id);
        self.shown.retain(|w| *w != id);
        self.scratch.retain(|w| *w != id);
        self.scratch.push(id);
        match name {
            Some(n) => self.scratch_names.insert(id, n.to_string()),
            None => self.scratch_names.remove(&id),
        };
    }

    /// How much of the screen each named scratchpad's window takes, where its line says.
    pub fn set_scratchpad_sizes(&mut self, sizes: HashMap<String, f64>) {
        self.scratch_sizes = sizes;
    }

    fn in_scratchpad(&self, id: WinId, name: Option<&str>) -> bool {
        self.scratch_names.get(&id).map(String::as_str) == name
    }

    /// A scratchpad's windows on show on the workspace in view.
    pub fn scratchpad_in_view(&self, name: Option<&str>) -> Vec<WinId> {
        self.active_workspace().map(|ws| ws.floating.iter().copied().filter(|w| self.shown.contains(w) && self.in_scratchpad(*w, name)).collect()).unwrap_or_default()
    }

    /// The scratchpad key: put away what is on show here; else show the next window put
    /// away, floating over this workspace; else bring over one on show somewhere else.
    pub fn toggle_scratchpad(&mut self, name: Option<&str>) -> Scratch {
        let here = self.scratchpad_in_view(name);
        if !here.is_empty() {
            for id in here {
                if let Some(ws) = self.ws() {
                    ws.remove(id);
                }
                self.shown.retain(|w| *w != id);
                self.scratch.push(id);
            }
            self.settle();
            return Scratch::Hidden;
        }
        let id = match self.scratch.iter().position(|w| self.in_scratchpad(*w, name)) {
            Some(i) => self.scratch.remove(i),
            None => match self.shown.iter().find(|w| self.in_scratchpad(**w, name)) {
                Some(id) => *id,
                None => return Scratch::Empty,
            },
        };
        if let Some((mi, wi)) = self.find(id) {
            self.mons[mi].wss[wi].remove(id);
        }
        if !self.shown.contains(&id) {
            self.shown.push(id);
        }
        let Some(ws) = self.ws() else { return Scratch::Empty };
        ws.floating.push(id);
        ws.float_focus = Some(id);
        self.settle();
        Scratch::Shown(id)
    }

    // ---- floating windows ----

    pub fn float(&self, id: WinId) -> Float {
        self.floats.get(&id).copied().unwrap_or_default()
    }

    /// Put a floating window somewhere, or give it a size; it stays there, and that size,
    /// through being put in a column and floated again.
    pub fn set_float(&mut self, id: WinId, f: Float) {
        self.floats.insert(id, Float { size: f.size.map(|(w, h)| (w.max(80), h.max(60))), ..f });
    }

    /// The part of the display a window's workspace has, panels left out.
    pub fn area_of(&self, id: WinId) -> Option<Rect> {
        self.find(id).map(|(mi, _)| self.mons[mi].area)
    }

    /// The size a floating window is asked to be. None: its own choice.
    pub fn float_size(&self, id: WinId) -> Option<(i32, i32)> {
        self.float(id).size.or_else(|| self.scratchpad_size(id))
    }

    /// The size a window shown from the scratchpad is asked to be.
    fn scratchpad_size(&self, id: WinId) -> Option<(i32, i32)> {
        if !self.shown.contains(&id) {
            return None;
        }
        let m = &self.mons[self.find(id)?.0];
        let by = self.scratch_names.get(&id).and_then(|n| self.scratch_sizes.get(n)).copied().unwrap_or(self.opts.scratchpad_size);
        Some((((m.area.w as f64 * by).round() as i32).max(1), ((m.area.h as f64 * by).round() as i32).max(1)))
    }

    pub fn toggle_floating(&mut self, id: WinId) {
        let Some((mi, wi)) = self.find(id) else { return };
        let width = self.opts.default_width;
        let ws = &mut self.mons[mi].wss[wi];
        if ws.floating.contains(&id) {
            // Into a column, it is a window like any other again.
            self.shown.retain(|w| *w != id);
            self.scratch_names.remove(&id);
            let ws = &mut self.mons[mi].wss[wi];
            ws.floating.retain(|w| *w != id);
            ws.float_focus = None;
            ws.insert_column(Column { wins: vec![id], active: 0, width, maximized: false, tabbed: false });
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
    /// display (from 0); the focus goes with it.
    pub fn move_to_workspace(&mut self, idx: usize, whole_column: bool) {
        self.move_to_workspace_ref(&WorkspaceRef::Index(idx), whole_column);
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

    // ---- fingers on the touchpad ----

    /// Fingers down: what is in view follows them from here.
    pub fn swipe_begin(&mut self) {
        self.swipe = Some(Swipe { moved: (0.0, 0.0), along: None });
    }

    /// The fingers moved by this much on the screen. Along the strip, the windows scroll
    /// with them; across it, the workspaces do.
    pub fn swipe_update(&mut self, dx: f64, dy: f64) {
        let Some(mut sw) = self.swipe else { return };
        let o = self.opts.clone();
        let Some(m) = self.mons.get_mut(self.active_mon) else { return };
        let vertical = m.wss.get(m.active).is_some_and(|ws| m.ws_vertical(ws, &o));
        sw.moved.0 += dx;
        sw.moved.1 += dy;
        // Which way they are going is clear once they have gone a little way.
        let (mut along_by, mut across_by) = if vertical { (dy, dx) } else { (dx, dy) };
        if sw.along.is_none() {
            let (a, c) = if vertical { (sw.moved.1, sw.moved.0) } else { (sw.moved.0, sw.moved.1) };
            if a.abs().max(c.abs()) < 8.0 {
                self.swipe = Some(sw);
                return;
            }
            sw.along = Some(a.abs() >= c.abs());
            (along_by, across_by) = (a, c);
        }
        if sw.along == Some(true) {
            let area = m.strip_area(vertical);
            let ws = &mut m.wss[m.active];
            if let Some(last) = ws.cols.len().checked_sub(1) {
                let room = (area.w - 2 * o.gaps_out) as f64;
                let end = (ws.col_x(last, &area, &o) + ws.col_width(last, &area, &o)) as f64;
                ws.view = (ws.view - along_by).clamp(0.0, (end - room).max(0.0));
                ws.view_to = ws.view;
            }
        } else {
            // Workspaces lie the other way from the windows of the display.
            let screen = if m.vertical(&o) { m.size.0 } else { m.size.1 }.max(1) as f64;
            m.pos = (m.pos - across_by / screen).clamp(0.0, (m.wss.len() - 1) as f64);
        }
        self.swipe = Some(sw);
    }

    /// Fingers up: the nearest column, or workspace, is the one that stays.
    pub fn swipe_end(&mut self) {
        let Some(sw) = self.swipe.take() else { return };
        let o = self.opts.clone();
        let Some(m) = self.mons.get_mut(self.active_mon) else { return };
        match sw.along {
            Some(true) => {
                let vertical = m.wss.get(m.active).is_some_and(|ws| m.ws_vertical(ws, &o));
                let area = m.strip_area(vertical);
                let ws = &mut m.wss[m.active];
                // The column with the focus if it is still whole in view; else the one
                // nearest the middle of what is.
                let room = (area.w - 2 * o.gaps_out) as f64;
                let whole = |ws: &Workspace, i: usize| {
                    let (x, w) = (ws.col_x(i, &area, &o) as f64, ws.col_width(i, &area, &o) as f64);
                    x >= ws.view - 0.5 && x + w <= ws.view + room + 0.5
                };
                if !ws.cols.is_empty() && !whole(ws, ws.active) {
                    let mid = ws.view + room / 2.0;
                    let near = (0..ws.cols.len()).min_by(|a, b| {
                        let d = |i: usize| (ws.col_x(i, &area, &o) as f64 + ws.col_width(i, &area, &o) as f64 / 2.0 - mid).abs();
                        d(*a).total_cmp(&d(*b))
                    });
                    ws.active = near.unwrap_or(ws.active);
                    ws.float_focus = None;
                    if ws.fullscreen.is_some() {
                        ws.fullscreen = None;
                    }
                }
            }
            Some(false) => m.active = (m.pos.round() as usize).min(m.wss.len() - 1),
            None => {}
        }
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
                if m.wss[i].is_empty() && m.wss[i].name.is_none() && i != m.active {
                    m.wss.remove(i);
                    if m.active > i {
                        m.active -= 1;
                        m.pos = (m.pos - 1.0).max(0.0);
                    }
                } else {
                    i += 1;
                }
            }
            if self.mons[mi].wss.last().is_none_or(|w| !w.is_empty() || w.name.is_some()) {
                let ws = self.new_ws();
                self.mons[mi].wss.push(ws);
            }
            let m = &mut self.mons[mi];
            m.active = m.active.min(m.wss.len() - 1);
            if !m.orphan {
                for ws in m.wss.iter_mut().filter(|w| (w.is_empty() && w.name.is_none()) || w.home.is_empty()) {
                    ws.home.clone_from(&m.name);
                }
            }
            for wi in 0..m.wss.len() {
                let area = m.strip_area(m.ws_vertical(&m.wss[wi], &o));
                m.wss[wi].settle_view(&area, &o);
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
                now.extend(tiles.iter().filter(|t| !t.floating).map(|t| (t.id, (ws.id, t.rect))));
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
        // Under the fingers, the workspaces are where the fingers put them.
        let held = self.swipe.is_some_and(|s| s.along == Some(false)).then_some(self.active_mon);
        for (mi, m) in self.mons.iter_mut().enumerate() {
            // Workspaces slide by whole screens, so ease in screen heights (widths, where
            // the windows scroll up and down and the workspaces go sideways).
            let h = if m.vertical(&self.opts) { m.size.0 } else { m.size.1 }.max(1) as f64;
            if held != Some(mi) {
                let mut p = m.pos * h;
                ease(&mut p, m.active as f64 * h);
                m.pos = p / h;
            }
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
        let vertical = m.ws_vertical(ws, o);
        let area = &m.strip_area(vertical);
        let sliding = view.is_some();
        let view = view.unwrap_or(0.0);
        let b = o.border;
        for (ci, col) in ws.cols.iter().enumerate() {
            let w = ws.col_width(ci, area, o);
            let x = area.x + o.gaps_out + ws.col_x(ci, area, o) - view.round() as i32;
            let n = col.wins.len() as i32;
            let room = area.h - 2 * o.gaps_out - o.gaps * (n - 1);
            // Tabs: a strip across the top, and under it every window has the whole column.
            let strip = o.tab_bar + (o.gaps / 2).max(2);
            let (heights, down) = if col.tabbed { (vec![(area.h - 2 * o.gaps_out - strip).max(1); col.wins.len()], strip) } else { self.column_heights(col, room) };
            let mut next = area.y + o.gaps_out + down;
            for (ri, id) in col.wins.iter().enumerate() {
                let (y, h) = (next, heights[ri]);
                if !col.tabbed {
                    next += h + o.gaps;
                }
                let active = focused && ws.float_focus.is_none() && ci == ws.active && ri == col.active;
                let mut rect = Rect::new(x + b, y + b, (w - 2 * b).max(1), (h - 2 * b).max(1));
                // Its piece of the strip, a little room between the pieces.
                let mut tab = col.tabbed.then(|| {
                    let edge = |i: i32| x + (w + TAB_GAP) * i / n;
                    Rect::new(edge(ri as i32), area.y + o.gaps_out, (edge(ri as i32 + 1) - TAB_GAP - edge(ri as i32)).max(1), o.tab_bar)
                });
                if vertical {
                    rect = turned(rect);
                    tab = tab.map(turned);
                }
                // The strip goes along with a column that is sliding to its place.
                if let (Some(t), Some(o)) = (&mut tab, self.offsets.get(id).filter(|_| sliding)) {
                    if vertical { t.y += o[1].round() as i32 } else { t.x += o[0].round() as i32 }
                }
                let off = self.offsets.get(id).filter(|_| sliding);
                if let Some(o) = off {
                    rect = Rect::new(rect.x + o[0].round() as i32, rect.y + o[1].round() as i32, (rect.w + o[2].round() as i32).max(1), (rect.h + o[3].round() as i32).max(1));
                }
                if ws.fullscreen != Some(*id) {
                    out.push(Tile { id: *id, rect, border: b > 0, active, floating: false, fullscreen: false, moving: off.is_some(), resizing: off.is_some_and(|o| o[2] != 0.0 || o[3] != 0.0), zoom: 1.0, tab, hidden: col.tabbed && ri != col.active });
                }
            }
        }
        let area = &m.area;
        for id in &ws.floating {
            let (w, h) = float_size(*id);
            let (w, h) = (w.clamp(1, area.w.max(1)), h.clamp(1, area.h.max(1)));
            // Where it was put, else the middle; and never so far off that it can't be reached.
            let f = self.float(*id);
            let (x, y) = match f.pos {
                Some((x, y)) => {
                    let (rw, rh) = f.size.unwrap_or((w, h));
                    (area.x + x + if f.hold.0 { rw - w } else { 0 }, area.y + y + if f.hold.1 { rh - h } else { 0 })
                }
                None => (area.x + (area.w - w) / 2, area.y + (area.h - h) / 2),
            };
            let x = x.min(area.x + area.w - HOLD).max(area.x + HOLD - w);
            let y = y.min(area.y + area.h - HOLD).max(area.y);
            let rect = Rect::new(x, y, w, h);
            if ws.fullscreen != Some(*id) {
                out.push(Tile { id: *id, rect, border: b > 0, active: focused && ws.float_focus == Some(*id), floating: true, fullscreen: false, moving: false, resizing: false, zoom: 1.0, tab: None, hidden: false });
            }
        }
        if let Some(id) = ws.fullscreen {
            // On the strip, the whole screen is where the strip is scrolled to: so a window
            // going to full screen, or back, slides from where it was seen.
            let by = if sliding { 0 } else { ws.view_to.round() as i32 };
            let mut rect = if vertical { Rect::new(0, by, m.size.0, m.size.1) } else { Rect::new(by, 0, m.size.0, m.size.1) };
            let off = self.offsets.get(&id).filter(|_| sliding);
            if let Some(o) = off {
                rect = Rect::new(rect.x + o[0].round() as i32, rect.y + o[1].round() as i32, (rect.w + o[2].round() as i32).max(1), (rect.h + o[3].round() as i32).max(1));
            }
            out.push(Tile { id, rect, border: false, active: focused, floating: false, fullscreen: true, moving: off.is_some(), resizing: off.is_some(), zoom: 1.0, tab: None, hidden: false });
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
            // Workspaces go the other way from the windows: one under the other where the
            // windows scroll sideways, side by side where they scroll up and down.
            let vertical = m.vertical(&self.opts);
            let (dx, dy) = if vertical { ((off * m.size.0 as f64).round() as i32, 0) } else { (0, (off * m.size.1 as f64).round() as i32) };
            let start = out.len();
            self.ws_tiles(m, ws, mon == self.active_mon && wi == m.active, float_size, Some(ws.view), &mut out);
            for t in &mut out[start..] {
                t.rect.x += dx;
                t.rect.y += dy;
                if let Some(tab) = &mut t.tab {
                    tab.x += dx;
                    tab.y += dy;
                }
            }
        }
        out.retain(|t| t.rect.intersects(&screen));
        out
    }

    /// The windows of every workspace of a display, each where it is on its own workspace
    /// (the strip's scroll applied, the slide between workspaces not), with the workspace's
    /// place in the stack.
    pub fn all_tiles(&self, mon: usize, float_size: &dyn Fn(WinId) -> (i32, i32)) -> Vec<(usize, Tile)> {
        let Some(m) = self.mons.get(mon) else { return Vec::new() };
        let mut out = Vec::new();
        let mut tiles = Vec::new();
        for (wi, ws) in m.wss.iter().enumerate() {
            tiles.clear();
            self.ws_tiles(m, ws, mon == self.active_mon && wi == m.active, float_size, Some(ws.view), &mut tiles);
            out.extend(tiles.iter().map(|t| (wi, *t)));
        }
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
                    let by = ws.view_to.round() as i32;
                    if m.ws_vertical(ws, &self.opts) { t.rect.y -= by } else { t.rect.x -= by }
                }
            }
        }
        out
    }

    /// The size a window about to open would get, so its first frame is already the right one.
    pub fn new_window_size(&self, on: Option<&str>) -> Option<(i32, i32)> {
        let m = on.and_then(|n| self.monitor(n)).and_then(|i| self.mons.get(i)).or(self.mons.get(self.active_mon))?;
        let o = &self.opts;
        let vertical = m.wss.get(m.active).is_some_and(|ws| m.ws_vertical(ws, o));
        let area = m.strip_area(vertical);
        let full = area.w - 2 * o.gaps_out;
        let w = (((full + o.gaps) as f64) * o.default_width).round() as i32 - o.gaps;
        let (w, h) = ((w - 2 * o.border).max(1), (area.h - 2 * o.gaps_out - 2 * o.border).max(1));
        Some(if vertical { (h, w) } else { (w, h) })
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
    fn windows_scroll_up_and_down() {
        let mut l = layout();
        l.set_options(Options { scroll: Scroll::Vertical, ..l.opts.clone() });
        for id in 1..=3 {
            l.add_window(id, false, None);
        }
        l.finish_animations();
        let rect = |l: &Layout, id| l.tiles(0, &fs).iter().find(|t| t.id == id).map(|t| t.rect);
        // Rows as wide as the screen, half its height each: 600 high, so 285 and a gap.
        // The third is in view, so the first has scrolled off the top.
        assert_eq!(rect(&l, 1), None);
        assert_eq!(rect(&l, 2), Some(Rect::new(12, 12, 976, 281)));
        assert_eq!(rect(&l, 3), Some(Rect::new(12, 307, 976, 281)));
        // Up is the row before, and the strip scrolls back down to show it.
        l.focus(Dir::Up);
        l.focus(Dir::Up);
        l.finish_animations();
        assert_eq!(l.active_window(), Some(1));
        assert_eq!(rect(&l, 1), Some(Rect::new(12, 12, 976, 281)));
        assert_eq!(rect(&l, 3), None);
        // Two windows in a row sit side by side, and left and right go between them.
        l.focus(Dir::Down);
        l.consume_or_expel(Dir::Left);
        l.finish_animations();
        assert_eq!(rect(&l, 1), Some(Rect::new(12, 12, 481, 281)));
        assert_eq!(rect(&l, 2), Some(Rect::new(507, 12, 481, 281)));
        l.focus(Dir::Left);
        assert_eq!(l.active_window(), Some(1));
        // A new window is asked to be the size of a row.
        assert_eq!(l.new_window_size(None), Some((976, 281)));
        // Sideways again, the same windows are columns.
        l.set_options(Options { scroll: Scroll::Horizontal, ..l.opts.clone() });
        l.finish_animations();
        assert_eq!(rect(&l, 1), Some(Rect::new(12, 12, 481, 281)));
        assert_eq!(rect(&l, 2), Some(Rect::new(12, 307, 481, 281)));
    }

    #[test]
    fn past_the_end_of_a_column_is_the_next_workspace() {
        let mut l = layout();
        l.add_window(1, false, None);
        l.add_window(2, false, None);
        l.consume_or_expel(Dir::Left);
        // Two windows in one column, the lower one focused: up is the window above, and up
        // again has nowhere to go; down from the lower one is the workspace below.
        l.focus_or_workspace(Dir::Up);
        assert_eq!(l.active_window(), Some(1));
        l.focus_or_workspace(Dir::Up);
        assert_eq!((l.mons[0].active, l.active_window()), (0, Some(1)));
        l.focus_or_workspace(Dir::Down);
        l.focus_or_workspace(Dir::Down);
        assert_eq!((l.mons[0].active, l.active_window()), (1, None));
        l.focus_or_workspace(Dir::Up);
        assert_eq!(l.mons[0].active, 0);
        // Every workspace's windows, for the overview: both of these on the first.
        assert_eq!(l.all_tiles(0, &fs).iter().map(|(ws, t)| (*ws, t.id)).collect::<Vec<_>>(), vec![(0, 1), (0, 2)]);
    }

    #[test]
    fn a_tabbed_column_shows_one_window() {
        let mut l = layout();
        l.add_window(1, false, None);
        l.add_window(2, false, None);
        l.consume_or_expel(Dir::Left);
        l.toggle_tabbed();
        l.finish_animations();
        let t = l.tiles(0, &fs);
        // Both have the whole column under the strip: 600 high, 10 at each edge, a strip of
        // 8 and 5 under it. The second, focused, is the one showing.
        assert_eq!(t.iter().map(|t| (t.id, t.rect, t.hidden)).collect::<Vec<_>>(), vec![(1, Rect::new(12, 25, 481, 563), true), (2, Rect::new(12, 25, 481, 563), false)]);
        // The strip: the column's 485 in two pieces with 4 between.
        assert_eq!(t.iter().map(|t| t.tab.unwrap()).collect::<Vec<_>>(), vec![Rect::new(10, 10, 240, 8), Rect::new(254, 10, 241, 8)]);
        // Up is the tab before.
        l.focus(Dir::Up);
        assert_eq!(l.active_window(), Some(1));
        assert!(l.tiles(0, &fs).iter().all(|t| t.hidden == (t.id == 2)));
        // And all showing again.
        l.toggle_tabbed();
        l.finish_animations();
        assert!(l.tiles(0, &fs).iter().all(|t| !t.hidden && t.tab.is_none() && t.rect.h == 281));
    }

    #[test]
    fn a_workspace_scrolls_its_own_way() {
        let mut l = layout();
        l.add_window(1, false, None);
        l.focus_workspace(1);
        l.add_window(2, false, None);
        // The second workspace up and down, the first left as it was.
        l.set_workspace_scroll(None);
        l.finish_animations();
        let size = |l: &Layout, id| l.sizes(0, &fs).iter().find(|t| t.id == id).map(|t| (t.rect.w, t.rect.h)).unwrap();
        assert_eq!((size(&l, 1), size(&l, 2)), ((481, 576), (976, 281)));
        // Down is along the strip here, and across it on the first.
        assert_eq!(l.along(true), Dir::Down);
        l.focus_workspace(0);
        assert_eq!(l.along(true), Dir::Right);
        // The display turned the other way takes only the workspace that wasn't told.
        l.set_options(Options { scroll: Scroll::Vertical, ..l.opts.clone() });
        assert_eq!((size(&l, 1), size(&l, 2)), ((976, 281), (976, 281)));
        l.focus_workspace(1);
        l.set_workspace_scroll(Some(Some(Scroll::Horizontal)));
        assert_eq!(size(&l, 2), (481, 576));
        l.set_workspace_scroll(Some(None));
        assert_eq!(size(&l, 2), (976, 281));
    }

    #[test]
    fn full_screen_grows_from_the_window_and_ends_when_the_focus_goes() {
        let mut l = layout();
        l.add_window(1, false, None);
        l.add_window(2, false, None);
        l.finish_animations();
        let tile = |l: &Layout, id| l.tiles(0, &fs).into_iter().find(|t| t.id == id).unwrap();
        let was = tile(&l, 2).rect;
        l.set_fullscreen(2, Some(true));
        // It starts where it stood, on its way to the whole screen.
        let t = tile(&l, 2);
        assert!(t.fullscreen && t.moving && t.rect == was);
        l.finish_animations();
        assert_eq!(tile(&l, 2).rect, Rect::new(0, 0, 1000, 600));
        assert_eq!(l.sizes(0, &fs).iter().find(|t| t.id == 2).map(|t| (t.rect.w, t.rect.h)), Some((1000, 600)));
        // The focus going to the window beside it ends it.
        l.focus(Dir::Left);
        assert_eq!((l.active_window(), tile(&l, 2).fullscreen), (Some(1), false));
        // And so does a window opening on the workspace.
        l.set_fullscreen(1, Some(true));
        l.add_window(3, false, None);
        assert_eq!((l.active_window(), tile(&l, 1).fullscreen), (Some(3), false));
        // Asked for on another display, it goes there.
        l.add_monitor("B", (800, 600));
        l.move_window_to_monitor(3, "B");
        l.set_fullscreen(3, Some(true));
        l.finish_animations();
        assert_eq!(l.tiles(1, &fs).iter().map(|t| (t.id, t.rect)).collect::<Vec<_>>(), vec![(3, Rect::new(0, 0, 800, 600))]);
    }

    #[test]
    fn a_window_is_put_down_by_another() {
        let mut l = layout();
        for id in 1..=3 {
            l.add_window(id, false, None);
        }
        let cols = |l: &Layout| l.active_workspace().unwrap().cols.iter().map(|c| c.wins.clone()).collect::<Vec<_>>();
        l.drop_window(3, Spot::Before(1));
        assert_eq!((cols(&l), l.active_window()), (vec![vec![3], vec![1], vec![2]], Some(3)));
        l.drop_window(3, Spot::Below(2));
        assert_eq!((cols(&l), l.active_window()), (vec![vec![1], vec![2, 3]], Some(3)));
        l.drop_window(2, Spot::Above(1));
        assert_eq!(cols(&l), vec![vec![2, 1], vec![3]]);
        l.drop_window(2, Spot::After(3));
        assert_eq!(cols(&l), vec![vec![1], vec![3], vec![2]]);
        // On itself, nothing; onto another display, its last column.
        l.drop_window(2, Spot::Above(2));
        assert_eq!(cols(&l), vec![vec![1], vec![3], vec![2]]);
        l.add_monitor("B", (800, 600));
        l.drop_window(1, Spot::Last(1));
        assert_eq!((l.active_mon, cols(&l)), (1, vec![vec![1]]));
    }

    #[test]
    fn a_tall_display_scrolls_up_and_down_when_left_to_choose() {
        let mut l = Layout::new(Options { scroll: Scroll::Auto, ..Options::default() });
        l.add_monitor("wide", (1000, 600));
        l.add_monitor("tall", (600, 1000));
        assert!(!l.mons[0].vertical(&l.opts));
        assert!(l.mons[1].vertical(&l.opts));
        // A display's own line says otherwise.
        l.set_scroll("tall", Some(Scroll::Horizontal));
        assert!(!l.mons[1].vertical(&l.opts));
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
        l.move_to_scratchpad(None);
        assert_eq!((l.find(2), l.active_window(), l.scratch.clone()), (None, Some(1), vec![2]));
        // Shown: floating, focused, at the scratchpad's size.
        assert_eq!(l.toggle_scratchpad(None), Scratch::Shown(2));
        assert_eq!((l.is_floating(2), l.active_window(), l.float_size(2)), (true, Some(2), Some((700, 420))));
        assert_eq!(l.float_size(1), None);
        // And away again, with the focus back where it was.
        assert_eq!(l.toggle_scratchpad(None), Scratch::Hidden);
        assert_eq!((l.find(2), l.active_window()), (None, Some(1)));
        // On show on one workspace, asked for on another: it comes over.
        l.toggle_scratchpad(None);
        l.focus_workspace_by(1);
        assert_eq!(l.toggle_scratchpad(None), Scratch::Shown(2));
        assert_eq!(l.find(2), Some((0, 1)));
        // Two put away take turns.
        l.toggle_scratchpad(None);
        l.focus_workspace_by(-1);
        l.move_to_scratchpad(None);
        assert_eq!(l.scratch, vec![2, 1]);
        assert_eq!(l.toggle_scratchpad(None), Scratch::Shown(2));
        l.toggle_scratchpad(None);
        assert_eq!(l.toggle_scratchpad(None), Scratch::Shown(1));
        // Put into a column, it is an ordinary window again; closed, it is forgotten.
        l.toggle_floating(1);
        assert_eq!((l.toggle_scratchpad(None), l.is_floating(1)), (Scratch::Shown(2), false));
        l.remove_window(2);
        assert_eq!(l.toggle_scratchpad(None), Scratch::Empty);
        assert!(l.scratch.is_empty());
    }

    #[test]
    fn a_floating_window_stays_where_it_is_put() {
        let mut l = layout();
        l.add_window(1, true, None);
        let rect = |l: &Layout| l.tiles(0, &fs)[0].rect;
        assert_eq!(rect(&l), Rect::new(350, 200, 300, 200));
        l.set_float(1, Float { pos: Some((20, 30)), size: Some((400, 10)), ..Default::default() });
        // Where it was put (its picture is still the size the window itself has), and the
        // size it is asked to be, which is never tiny.
        assert_eq!((rect(&l), l.float_size(1)), (Rect::new(20, 30, 300, 200), Some((400, 60))));
        // Through a column and back.
        l.toggle_floating(1);
        l.toggle_floating(1);
        assert_eq!(rect(&l), Rect::new(20, 30, 300, 200));
        // Dragged off the display, enough stays on to take hold of.
        l.set_float(1, Float { pos: Some((5000, -500)), size: None, ..Default::default() });
        assert_eq!(rect(&l), Rect::new(952, 0, 300, 200));
        l.set_float(1, Float { pos: Some((-5000, 5000)), size: None, ..Default::default() });
        assert_eq!(rect(&l), Rect::new(-252, 552, 300, 200));
        // Resized by its left and top edges, its far corner stays where it is while the
        // window itself is still the old size.
        l.set_float(1, Float { pos: Some((100, 100)), size: Some((400, 300)), hold: (true, true) });
        assert_eq!(rect(&l), Rect::new(200, 200, 300, 200));
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

    fn names(l: &Layout, mon: usize) -> Vec<Option<&str>> {
        l.mons[mon].wss.iter().map(|w| w.name.as_deref()).collect()
    }

    #[test]
    fn named_workspaces_stay_and_are_found_by_name() {
        let mut l = layout();
        l.add_window(1, false, None);
        l.set_named_workspaces(&[("chat".into(), None, Some(Scroll::Vertical)), ("mail".into(), Some("B".into()), None)]);
        // First in the stack, in the settings' order; what was in view still is.
        assert_eq!(names(&l, 0), vec![Some("chat"), Some("mail"), None, None]);
        assert_eq!((l.mons[0].active, l.active_window()), (2, Some(1)));
        // Empty, and it stays; and it scrolls as its line says.
        l.focus_workspace_ref(&WorkspaceRef::Name("chat".into()));
        assert_eq!((l.mons[0].active, l.vertical()), (0, true));
        l.focus_workspace(2);
        l.move_to_workspace_ref(&WorkspaceRef::Name("mail".into()), true);
        assert_eq!((l.mons[0].active, l.active_window()), (1, Some(1)));
        assert_eq!(names(&l, 0), vec![Some("chat"), Some("mail"), None]);
        // Its display arrives: it goes there with what is on it, and can be gone to by name.
        l.add_monitor("B", (800, 600));
        l.set_named_workspaces(&[("chat".into(), None, None), ("mail".into(), Some("B".into()), None)]);
        assert_eq!((names(&l, 0), names(&l, 1)), (vec![Some("chat"), None], vec![Some("mail"), None]));
        l.active_mon = 0;
        l.focus_workspace_ref(&WorkspaceRef::Name("mail".into()));
        assert_eq!((l.active_mon, l.active_window()), (1, Some(1)));
        // Its line goes: it is a workspace like any other, and goes when it is empty.
        l.set_named_workspaces(&[("mail".into(), Some("B".into()), None)]);
        assert_eq!(names(&l, 0), vec![None]);
    }

    #[test]
    fn a_window_opens_where_and_how_it_is_told() {
        let mut l = layout();
        l.set_named_workspaces(&[("chat".into(), None, None)]);
        l.focus_workspace(1);
        l.add_window(1, false, None);
        // On another workspace without the focus: nothing in view changes.
        let chat = WorkspaceRef::Name("chat".into());
        l.open_window(2, Open { workspace: Some(&chat), focus: false, width: Some(0.25), ..Default::default() });
        assert_eq!((l.mons[0].active, l.active_window(), l.find(2)), (1, Some(1), Some((0, 0))));
        assert_eq!(l.mons[0].wss[0].cols[0].width, 0.25);
        // Beside the focused window without taking the focus from it.
        l.open_window(3, Open { focus: false, ..Default::default() });
        assert_eq!((l.active_window(), l.position(3)), (Some(1), Some((2, 1))));
        // And with the focus, the workspace it names comes into view.
        l.open_window(4, Open { workspace: Some(&chat), floating: true, ..Default::default() });
        assert_eq!((l.mons[0].active, l.active_window(), l.is_floating(4)), (0, Some(4), true));
    }

    #[test]
    fn each_scratchpad_has_its_own_windows() {
        let mut l = layout();
        l.set_scratchpad_sizes([("term".to_string(), 0.5)].into_iter().collect());
        l.add_window(1, false, None);
        l.add_window(2, false, None);
        l.move_to_scratchpad(Some("term"));
        // A window that opens put away.
        l.put_away(3, Some("notes"));
        assert_eq!(l.toggle_scratchpad(None), Scratch::Empty);
        assert_eq!(l.toggle_scratchpad(Some("notes")), Scratch::Shown(3));
        assert_eq!(l.toggle_scratchpad(Some("term")), Scratch::Shown(2));
        assert_eq!((l.float_size(2), l.float_size(3)), (Some((500, 300)), Some((700, 420))));
        // One key puts its own away and leaves the other's on show.
        assert_eq!(l.toggle_scratchpad(Some("notes")), Scratch::Hidden);
        assert_eq!((l.scratchpad_in_view(Some("term")), l.scratchpad_in_view(Some("notes"))), (vec![2], vec![]));
        assert_eq!(l.toggle_scratchpad(Some("missing")), Scratch::Empty);
    }

    #[test]
    fn fingers_scroll_the_strip_and_the_workspaces() {
        let mut l = layout();
        for id in 1..=4 {
            l.add_window(id, false, None);
        }
        l.focus_column_edge(false);
        l.finish_animations();
        assert_eq!((l.active_window(), l.mons[0].wss[0].view), (Some(1), 0.0));
        // Fingers to the left: the strip follows, and the nearest column takes the focus.
        l.swipe_begin();
        l.swipe_update(-30.0, 2.0);
        l.swipe_update(-900.0, 0.0);
        assert_eq!(l.mons[0].wss[0].view, 930.0);
        assert!(!l.advance(0.016) || l.mons[0].wss[0].view == 930.0);
        l.swipe_end();
        assert_eq!(l.active_window(), Some(3));
        // It can't be pulled past the end, and a column still whole in view keeps the focus.
        l.swipe_begin();
        l.swipe_update(-9000.0, 0.0);
        let end = l.mons[0].wss[0].view;
        l.swipe_end();
        assert_eq!((l.active_window(), end), (Some(3), 990.0));
        // Fingers up: the workspace below comes with them, and stays when let go past half.
        l.finish_animations();
        l.swipe_begin();
        l.swipe_update(0.0, -400.0);
        assert!((l.mons[0].pos - 400.0 / 600.0).abs() < 1e-9);
        l.advance(0.5);
        assert!((l.mons[0].pos - 400.0 / 600.0).abs() < 1e-9);
        l.swipe_end();
        assert_eq!(l.mons[0].active, 1);
        // Not far enough: back where it was.
        l.finish_animations();
        l.swipe_begin();
        l.swipe_update(0.0, 200.0);
        l.swipe_end();
        assert_eq!(l.mons[0].active, 1);
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
        l.finish_animations();
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
