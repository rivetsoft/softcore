//! Softcore's settings: one `key=value` file that people and a settings window both edit.
//!
//! - Every plain setting is in [`SETTINGS`], with its type, range, default, label and group:
//!   the reader checks values against it and a window can be drawn from it.
//! - Things there are many of are keyed by name, so each has one line that can be found and
//!   changed: `bind.Mod+T=spawn foot`, `output.DP-5=scale=1.5`, `env.NAME=VALUE`.
//! - [`SettingsFile`] changes one line in place and leaves comments, order and lines it
//!   doesn't know alone, so a change made in a window and one made in an editor never undo
//!   each other.
//!
//! Nothing here depends on the compositor or on a toolkit: Softcore reads with it, a settings
//! window reads and writes with it.

mod file;
mod schema;

pub use file::SettingsFile;
pub use schema::{Config, Group, Kind, SETTINGS, Setting, check, config_file, setting};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

/// What the settings give the layout.
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// Between windows.
    pub gaps: i32,
    /// Between windows and the edges of the screen.
    pub gaps_out: i32,
    /// The border's width, drawn outside the window.
    pub border: i32,
    /// Column widths that `switch-preset-column-width` steps through, as parts of the screen.
    pub presets: Vec<f64>,
    /// A new column's width.
    pub default_width: f64,
    /// How fast things slide into place: 1 is usual, 2 twice as fast, 0 no sliding at all.
    pub animation_speed: f64,
    /// A column whose windows don't fill the screen's height sits in the middle of it
    /// (else at the top).
    pub center_short_columns: bool,
    /// How much of the screen a window shown from the scratchpad takes, each way.
    pub scratchpad_size: f64,
    /// Which way the strip of windows runs.
    pub scroll: Scroll,
    /// How thick the strip of tabs over a tabbed column is.
    pub tab_bar: i32,
}

/// Which way a strip of windows runs, and so which way it scrolls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scroll {
    /// Columns side by side, scrolling left and right.
    #[default]
    Horizontal,
    /// Rows one under the other, scrolling up and down.
    Vertical,
    /// Up and down on a display taller than it is wide, sideways on the others.
    Auto,
}

impl Scroll {
    pub fn parse(v: &str) -> Option<Scroll> {
        match v {
            "horizontal" => Some(Scroll::Horizontal),
            "vertical" => Some(Scroll::Vertical),
            "auto" => Some(Scroll::Auto),
            _ => None,
        }
    }
}

impl Default for Options {
    fn default() -> Self {
        Options { gaps: 12, gaps_out: 12, border: 3, presets: vec![1.0 / 3.0, 0.5, 2.0 / 3.0], default_width: 0.5, animation_speed: 1.0, center_short_columns: true, scratchpad_size: 0.7, scroll: Scroll::Horizontal, tab_bar: 8 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Spawn(String),
    Quit { confirm: bool },
    CloseWindow,
    Focus(Dir),
    FocusColumnFirst,
    FocusColumnLast,
    MoveColumn(Dir),
    ConsumeOrExpel(Dir),
    SwitchPresetWidth,
    SetColumnWidth(WidthChange),
    SetWindowHeight(WidthChange),
    ResetWindowHeight,
    MaximizeColumn,
    /// A column's windows one over the other, one showing at a time, or side by side again.
    ToggleTabbed,
    FullscreenWindow,
    CenterColumn,
    ToggleFloating,
    MoveToScratchpad,
    ToggleScratchpad,
    SwitchFloatingFocus,
    FocusWorkspace(usize),
    FocusWorkspaceBy(i32),
    MoveToWorkspace { idx: usize, column: bool },
    MoveToWorkspaceBy { by: i32, column: bool },
    FocusMonitor(i32),
    MoveColumnToMonitor(i32),
    ShowHotkeyOverlay,
    /// Every workspace at once, drawn small. None: open if shut, shut if open.
    Overview(Option<bool>),
    ToggleShortcutsInhibit,
    PowerOffMonitors,
    PowerOnMonitors,
    ReloadConfig,
    Screenshot(Option<String>),
    ChangeVt(i32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WidthChange {
    /// Part of the screen, 0–1.
    To(f64),
    By(f64),
}

impl Action {
    /// From its name and what follows it.
    pub fn parse(text: &str) -> Result<Action, String> {
        let text = text.trim();
        let (name, rest) = text.split_once(char::is_whitespace).map(|(a, b)| (a, b.trim())).unwrap_or((text, ""));
        let num = |what: &str| rest.parse::<usize>().ok().filter(|n| *n >= 1).ok_or(format!("{name} needs {what}"));
        Ok(match name {
            "spawn" | "spawn-sh" if !rest.is_empty() => Action::Spawn(rest.to_string()),
            "spawn" | "spawn-sh" => return Err("spawn needs a command".into()),
            "quit" => Action::Quit { confirm: rest != "--skip-confirmation" && rest != "now" },
            "close-window" => Action::CloseWindow,
            "focus-column-left" => Action::Focus(Dir::Left),
            "focus-column-right" => Action::Focus(Dir::Right),
            "focus-window-up" => Action::Focus(Dir::Up),
            "focus-window-down" => Action::Focus(Dir::Down),
            "focus-column-first" => Action::FocusColumnFirst,
            "focus-column-last" => Action::FocusColumnLast,
            "move-column-left" => Action::MoveColumn(Dir::Left),
            "move-column-right" => Action::MoveColumn(Dir::Right),
            "move-window-up" => Action::MoveColumn(Dir::Up),
            "move-window-down" => Action::MoveColumn(Dir::Down),
            "consume-or-expel-window-left" => Action::ConsumeOrExpel(Dir::Left),
            "consume-or-expel-window-right" => Action::ConsumeOrExpel(Dir::Right),
            "switch-preset-column-width" => Action::SwitchPresetWidth,
            "set-column-width" => Action::SetColumnWidth(parse_width(name, rest)?),
            "set-window-height" => Action::SetWindowHeight(parse_width(name, rest)?),
            "reset-window-height" => Action::ResetWindowHeight,
            "maximize-column" => Action::MaximizeColumn,
            "toggle-column-tabbed-display" => Action::ToggleTabbed,
            "fullscreen-window" => Action::FullscreenWindow,
            "center-column" => Action::CenterColumn,
            "toggle-window-floating" => Action::ToggleFloating,
            "move-window-to-scratchpad" => Action::MoveToScratchpad,
            "toggle-scratchpad" => Action::ToggleScratchpad,
            "switch-focus-between-floating-and-tiling" => Action::SwitchFloatingFocus,
            "focus-workspace" => Action::FocusWorkspace(num("a workspace number")? - 1),
            "focus-workspace-down" => Action::FocusWorkspaceBy(1),
            "focus-workspace-up" => Action::FocusWorkspaceBy(-1),
            "move-column-to-workspace" => Action::MoveToWorkspace { idx: num("a workspace number")? - 1, column: true },
            "move-window-to-workspace" => Action::MoveToWorkspace { idx: num("a workspace number")? - 1, column: false },
            "move-column-to-workspace-down" => Action::MoveToWorkspaceBy { by: 1, column: true },
            "move-column-to-workspace-up" => Action::MoveToWorkspaceBy { by: -1, column: true },
            "move-window-to-workspace-down" => Action::MoveToWorkspaceBy { by: 1, column: false },
            "move-window-to-workspace-up" => Action::MoveToWorkspaceBy { by: -1, column: false },
            "focus-monitor-left" | "focus-monitor-previous" => Action::FocusMonitor(-1),
            "focus-monitor-right" | "focus-monitor-next" => Action::FocusMonitor(1),
            "move-column-to-monitor-left" | "move-column-to-monitor-previous" => Action::MoveColumnToMonitor(-1),
            "move-column-to-monitor-right" | "move-column-to-monitor-next" => Action::MoveColumnToMonitor(1),
            "show-hotkey-overlay" => Action::ShowHotkeyOverlay,
            "toggle-overview" => Action::Overview(None),
            "open-overview" => Action::Overview(Some(true)),
            "close-overview" => Action::Overview(Some(false)),
            "toggle-keyboard-shortcuts-inhibit" => Action::ToggleShortcutsInhibit,
            "power-off-monitors" => Action::PowerOffMonitors,
            "power-on-monitors" => Action::PowerOnMonitors,
            "reload-config" | "load-config-file" => Action::ReloadConfig,
            "change-vt" => Action::ChangeVt(num("a TTY's number")? as i32),
            "screenshot" => Action::Screenshot((!rest.is_empty()).then(|| rest.to_string())),
            _ => return Err(format!("no action called {name}")),
        })
    }

    /// What the keys panel calls it.
    pub fn label(&self) -> String {
        match self {
            Action::Spawn(c) => format!("Run {c}"),
            Action::Quit { .. } => "Leave Softcore".into(),
            Action::CloseWindow => "Close the window".into(),
            Action::Focus(d) => format!("Focus {}", dir_name(*d)),
            Action::FocusColumnFirst => "Focus the first column".into(),
            Action::FocusColumnLast => "Focus the last column".into(),
            Action::MoveColumn(d) => format!("Move {}", dir_name(*d)),
            Action::ConsumeOrExpel(d) => format!("Into or out of the column on the {}", dir_name(*d)),
            Action::SwitchPresetWidth => "Next column width".into(),
            Action::SetColumnWidth(WidthChange::By(b)) if *b < 0.0 => "Narrower column".into(),
            Action::SetColumnWidth(_) => "Wider column".into(),
            Action::SetWindowHeight(WidthChange::By(b)) if *b < 0.0 => "Shorter window".into(),
            Action::SetWindowHeight(_) => "Taller window".into(),
            Action::ResetWindowHeight => "Window as tall as there is room".into(),
            Action::MaximizeColumn => "Column as wide as the screen".into(),
            Action::ToggleTabbed => "The windows of the column as tabs, or all showing".into(),
            Action::FullscreenWindow => "Full screen".into(),
            Action::CenterColumn => "Centre the column".into(),
            Action::ToggleFloating => "Float the window, or put it back".into(),
            Action::MoveToScratchpad => "Put the window away in the scratchpad".into(),
            Action::ToggleScratchpad => "Show the scratchpad, or put it away".into(),
            Action::SwitchFloatingFocus => "Between floating windows and columns".into(),
            Action::FocusWorkspace(n) => format!("Workspace {}", n + 1),
            Action::FocusWorkspaceBy(b) => format!("Workspace {}", if *b > 0 { "below" } else { "above" }),
            Action::MoveToWorkspace { idx, .. } => format!("Move to workspace {}", idx + 1),
            Action::MoveToWorkspaceBy { by, .. } => format!("Move to the workspace {}", if *by > 0 { "below" } else { "above" }),
            Action::FocusMonitor(b) => format!("{} display", if *b > 0 { "Next" } else { "Previous" }),
            Action::MoveColumnToMonitor(b) => format!("Move to the {} display", if *b > 0 { "next" } else { "previous" }),
            Action::ShowHotkeyOverlay => "Show these keys".into(),
            Action::Overview(None) => "Overview: every workspace at once".into(),
            Action::Overview(Some(true)) => "Open the overview".into(),
            Action::Overview(Some(false)) => "Close the overview".into(),
            Action::ToggleShortcutsInhibit => "Give this window every key, or take them back".into(),
            Action::PowerOffMonitors => "Displays off".into(),
            Action::PowerOnMonitors => "Displays on".into(),
            Action::ReloadConfig => "Read the settings again".into(),
            Action::Screenshot(_) => "Screenshot".into(),
            Action::ChangeVt(n) => format!("TTY {n}"),
        }
    }
}

fn dir_name(d: Dir) -> &'static str {
    match d {
        Dir::Left => "left",
        Dir::Right => "right",
        Dir::Up => "up",
        Dir::Down => "down",
    }
}

fn parse_width(name: &str, t: &str) -> Result<WidthChange, String> {
    let bad = || format!("{name} needs a size like 50%, +10% or -10%, not {t:?}");
    let pct = t.strip_suffix('%').ok_or_else(bad)?;
    let v: f64 = pct.parse().map_err(|_| bad())?;
    Ok(if pct.starts_with('+') || pct.starts_with('-') { WidthChange::By(v / 100.0) } else { WidthChange::To(v / 100.0) })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Mods {
    /// The window-manager key: Super on a real screen, Alt in a window (see `mod_key`).
    pub modkey: bool,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub logo: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bind {
    pub mods: Mods,
    /// The key's xkb name, lower case ("return", "left", "xf86audiomute").
    pub key: String,
    /// As written, for the keys panel.
    pub shown: String,
    pub action: Action,
    /// Works while the screen is locked.
    pub locked: bool,
}

/// `Mod+Shift+Q` into its parts.
pub fn parse_combo(text: &str) -> Result<(Mods, String), String> {
    let mut mods = Mods::default();
    let mut key = None;
    for part in text.split('+').map(str::trim) {
        match part.to_ascii_lowercase().as_str() {
            "mod" => mods.modkey = true,
            "ctrl" | "control" => mods.ctrl = true,
            "shift" => mods.shift = true,
            "alt" => mods.alt = true,
            "super" | "win" | "logo" => mods.logo = true,
            "" => return Err(format!("{text:?} isn't a key")),
            k => key = Some(k.to_string()),
        }
    }
    key.map(|k| (mods, k)).ok_or(format!("{text:?} names no key"))
}

/// A binding from its keys and what follows them.
pub fn parse_bind(combo: &str, action: &str) -> Result<Bind, String> {
    let (mods, key) = parse_combo(combo)?;
    let mut action = action.trim();
    let mut locked = false;
    if let Some(a) = action.strip_prefix("allow-when-locked ") {
        locked = true;
        action = a.trim();
    }
    Ok(Bind { mods, key, shown: combo.to_string(), action: Action::parse(action)?, locked })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModKey {
    /// Super on a real screen, Alt when Softcore runs in a window of another desktop (whose
    /// own Super bindings would get there first).
    Auto,
    Super,
    Alt,
}

/// A display's settings: `output.NAME=mode=2560x1440@59.951 scale=1 position=0,0`, or
/// `output.NAME=off`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct OutputRule {
    pub name: String,
    pub off: bool,
    /// Width, height and, when given, the refresh rate in mHz.
    pub mode: Option<(i32, i32, Option<i32>)>,
    pub scale: Option<f64>,
    pub position: Option<(i32, i32)>,
    /// Turned by 0, 90, 180 or 270 degrees, and mirrored. None: as it is.
    pub transform: Option<(u32, bool)>,
    /// Which way this display's windows scroll. None: as the `scroll` setting says.
    pub scroll: Option<Scroll>,
}

/// A display's line: its name, and what follows it.
pub fn parse_output(name: &str, value: &str) -> Result<OutputRule, String> {
    let parts = value.split_whitespace();
    let mut r = OutputRule { name: name.to_string(), ..Default::default() };
    for p in parts {
        let bad = || format!("output.{name}={value}: couldn't read {p:?}");
        match p.split_once('=') {
            None if p == "off" => r.off = true,
            Some(("mode", m)) => {
                let (size, hz) = m.split_once('@').map(|(s, h)| (s, Some(h))).unwrap_or((m, None));
                let (w, h) = size.split_once('x').ok_or_else(bad)?;
                let hz = match hz {
                    Some(h) => Some((h.parse::<f64>().map_err(|_| bad())? * 1000.0).round() as i32),
                    None => None,
                };
                r.mode = Some((w.parse().map_err(|_| bad())?, h.parse().map_err(|_| bad())?, hz));
            }
            Some(("scale", s)) => r.scale = Some(s.parse::<f64>().ok().filter(|s| (0.5..=4.0).contains(s)).ok_or_else(bad)?),
            Some(("position", s)) => {
                let (x, y) = s.split_once(',').ok_or_else(bad)?;
                r.position = Some((x.parse().map_err(|_| bad())?, y.parse().map_err(|_| bad())?));
            }
            // normal, 90, 180, 270, flipped, flipped-90, flipped-180, flipped-270.
            Some(("transform", t)) => {
                let (flip, turn) = match t.strip_prefix("flipped") {
                    Some(rest) => (true, rest.trim_start_matches('-')),
                    None => (false, t),
                };
                let turn = match turn {
                    "" | "normal" => 0,
                    "90" => 90,
                    "180" => 180,
                    "270" => 270,
                    _ => return Err(bad()),
                };
                r.transform = Some((turn, flip));
            }
            Some(("scroll", v)) => r.scroll = Some(Scroll::parse(v).ok_or_else(bad)?),
            _ => return Err(bad()),
        }
    }
    Ok(r)
}


impl Mods {
    /// The one spelling of a key combination, so `mod+shift+q` and `Shift+Mod+Q` are the
    /// same binding: `Mod+Ctrl+Shift+Alt+Super+key`.
    pub fn combo(&self, key: &str) -> String {
        let mut out = String::new();
        for (on, name) in [(self.modkey, "Mod"), (self.ctrl, "Ctrl"), (self.shift, "Shift"), (self.alt, "Alt"), (self.logo, "Super")] {
            if on {
                out.push_str(name);
                out.push('+');
            }
        }
        out.push_str(key);
        out
    }
}

/// The bindings Softcore starts with, as `KEYS ACTION` lines. `default_binds=false` drops
/// them; a `bind.KEYS` line replaces one, and an empty one (`bind.Mod+E=`) removes it.

pub const DEFAULT_BINDS: &str = "\
Mod+Return spawn rivet-terminal
Mod+Space spawn rivet-shell msg launcher
Mod+E spawn rivet
Mod+Q close-window
Mod+Left focus-column-left
Mod+Right focus-column-right
Mod+Up focus-window-up
Mod+Down focus-window-down
Mod+H focus-column-left
Mod+L focus-column-right
Mod+K focus-window-up
Mod+J focus-window-down
Mod+Home focus-column-first
Mod+End focus-column-last
Mod+Ctrl+Left move-column-left
Mod+Ctrl+Right move-column-right
Mod+Ctrl+Up move-window-up
Mod+Ctrl+Down move-window-down
Mod+BracketLeft consume-or-expel-window-left
Mod+BracketRight consume-or-expel-window-right
Mod+R switch-preset-column-width
Mod+Minus set-column-width -10%
Mod+Equal set-column-width +10%
Mod+Shift+Minus set-window-height -10%
Mod+Shift+Equal set-window-height +10%
Mod+Ctrl+R reset-window-height
Mod+F maximize-column
Mod+W toggle-column-tabbed-display
Mod+Shift+F fullscreen-window
Mod+C center-column
Mod+V toggle-window-floating
Mod+Shift+V switch-focus-between-floating-and-tiling
Mod+S toggle-scratchpad
Mod+O toggle-overview
Mod+Shift+S move-window-to-scratchpad
Mod+Page_Down focus-workspace-down
Mod+Page_Up focus-workspace-up
Mod+U focus-workspace-down
Mod+I focus-workspace-up
Mod+Ctrl+Page_Down move-column-to-workspace-down
Mod+Ctrl+Page_Up move-column-to-workspace-up
Mod+1 focus-workspace 1
Mod+2 focus-workspace 2
Mod+3 focus-workspace 3
Mod+4 focus-workspace 4
Mod+5 focus-workspace 5
Mod+6 focus-workspace 6
Mod+7 focus-workspace 7
Mod+8 focus-workspace 8
Mod+9 focus-workspace 9
Mod+Ctrl+1 move-column-to-workspace 1
Mod+Ctrl+2 move-column-to-workspace 2
Mod+Ctrl+3 move-column-to-workspace 3
Mod+Ctrl+4 move-column-to-workspace 4
Mod+Ctrl+5 move-column-to-workspace 5
Mod+Ctrl+6 move-column-to-workspace 6
Mod+Ctrl+7 move-column-to-workspace 7
Mod+Ctrl+8 move-column-to-workspace 8
Mod+Ctrl+9 move-column-to-workspace 9
Mod+Shift+Left focus-monitor-left
Mod+Shift+Right focus-monitor-right
Mod+Shift+Ctrl+Left move-column-to-monitor-left
Mod+Shift+Ctrl+Right move-column-to-monitor-right
Mod+Shift+Slash show-hotkey-overlay
Mod+Shift+Escape toggle-keyboard-shortcuts-inhibit
Mod+Shift+E quit
Mod+Escape allow-when-locked spawn rivet-shell msg lock
XF86AudioRaiseVolume allow-when-locked spawn rivet-shell msg volume up
XF86AudioLowerVolume allow-when-locked spawn rivet-shell msg volume down
XF86AudioMute allow-when-locked spawn rivet-shell msg volume mute
XF86AudioMicMute allow-when-locked spawn rivet-shell msg mic mute
XF86MonBrightnessUp allow-when-locked spawn rivet-shell msg brightness up
XF86MonBrightnessDown allow-when-locked spawn rivet-shell msg brightness down
XF86AudioPlay allow-when-locked spawn rivet-shell msg media play-pause
XF86AudioNext allow-when-locked spawn rivet-shell msg media next
XF86AudioPrev allow-when-locked spawn rivet-shell msg media previous
";

pub fn parse_color(v: &str) -> Option<[u8; 3]> {
    let h = v.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(h, 16).ok()?;
    Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn the_schema_is_the_defaults() {
        let c = Config::default();
        for s in SETTINGS {
            assert_eq!(c.value(s.key), Some(s.default), "{}", s.key);
            assert!(check(s, s.default).is_ok(), "{}", s.key);
        }
        assert_eq!((c.layout.gaps, c.layout.gaps_out, c.layout.border), (12, 12, 3));
        assert_eq!(c.accent, [0x35, 0x84, 0xe4]);
        assert!(c.binds.len() > 50 && c.errors.is_empty());
    }

    #[test]
    fn reads_values_and_reports_bad_ones() {
        let c = Config::from_text("# look\ngaps=8\nborder=2\nradius=10\nborder_active=#e66100\nfuture_key=1\ncolumn_widths=0.5, 0.25\ntheme=dark\n");
        assert_eq!((c.layout.gaps, c.layout.gaps_out, c.layout.border, c.radius), (8, 8, 2, Some(10)));
        assert_eq!((c.border_active, c.theme), (Some([0xe6, 0x61, 0x00]), Theme::Dark));
        assert_eq!(c.layout.presets, vec![0.25, 0.5]);
        assert!(c.errors.is_empty());
        // The edges can be set apart, in either order.
        assert_eq!(Config::from_text("gaps_out=20\ngaps=6\n").layout.gaps_out, 20);
        let bad = Config::from_text("gaps=many\nborder=4\nbind.Mod+T=fly\nnonsense\ntheme=purple\n");
        assert_eq!((bad.layout.border, bad.layout.gaps, bad.errors.len()), (4, 12, 4));
        assert!(bad.errors[0].contains("a whole number from 0 to 200"));
    }

    #[test]
    fn bindings_outputs_and_env_are_keyed() {
        let c = Config::from_text("bind.Mod+Q=spawn foot\nbind.mod+e=\nbind.Shift+Mod+T=allow-when-locked spawn true\noutput.DP-5=mode=2560x1440@59.951 scale=1.5 position=2560,0\noutput.eDP-1=off\nenv.A=1\nenv.A=2\nspawn=rivet-shell\n");
        let q: Vec<&Bind> = c.binds.iter().filter(|b| b.key == "q").collect();
        assert_eq!((q.len(), &q[0].action), (1, &Action::Spawn("foot".into())));
        assert!(!c.binds.iter().any(|b| b.key == "e" && !b.mods.shift));
        assert!(c.binds.last().unwrap().locked && c.binds.last().unwrap().mods.shift);
        assert_eq!((c.outputs[0].mode, c.outputs[0].scale, c.outputs[0].position), (Some((2560, 1440, Some(59951))), Some(1.5), Some((2560, 0))));
        assert!(c.outputs[1].off);
        assert_eq!((c.env.clone(), c.spawn.clone()), (vec![("A".to_string(), "2".to_string())], vec!["rivet-shell".to_string()]));
        // Wherever default_binds=false stands, the file's own bindings stay.
        let only = Config::from_text("bind.Mod+T=spawn foot\ndefault_binds=false\n");
        assert_eq!(only.binds.len(), 1);
    }

    #[test]
    fn actions_parse() {
        assert_eq!(Action::parse("focus-workspace 3"), Ok(Action::FocusWorkspace(2)));
        assert_eq!(Action::parse("set-column-width -10%"), Ok(Action::SetColumnWidth(WidthChange::By(-0.1))));
        assert_eq!(Action::parse("quit --skip-confirmation"), Ok(Action::Quit { confirm: false }));
        assert!(Action::parse("focus-workspace").is_err());
    }

    #[test]
    fn a_change_touches_one_line_and_nothing_else() {
        let text = "# My Softcore\n\n# Look\ngaps=8\nborder=2   \n\n# Keys\nbind.Mod+T=spawn foot\nsome_future_key=kept\n";
        let mut f = SettingsFile::from_text(Path::new("settings"), text);
        f.set("border", "5");
        f.set("bind.mod+t", "spawn kitty");
        f.set("radius", "14");
        assert_eq!(f.text(), "# My Softcore\n\n# Look\ngaps=8\nborder=5\n\n# Keys\nbind.mod+t=spawn kitty\nsome_future_key=kept\nradius=14\n");
        assert_eq!((f.get("border"), f.get("bind.Mod+T"), f.get("shadows")), (Some("5"), Some("spawn kitty"), None));
        assert!(f.unset("radius") && !f.unset("radius"));
        // What the window wrote is what Softcore reads, and back.
        let c = Config::from_text(&f.text());
        assert_eq!((c.value("border"), c.layout.gaps), (Some("5"), 8));
        assert_eq!(f.entries("bind."), vec![("bind.mod+t".to_string(), "spawn kitty".to_string())]);
    }

    #[test]
    fn a_change_wins_over_an_included_file_and_repeats_collapse() {
        let mut f = SettingsFile::from_text(Path::new("settings"), "gaps=4\ngaps=6\ninclude=other.conf\nborder=2\n");
        f.set("border", "3");
        f.set("gaps", "9");
        assert_eq!(f.text(), "include=other.conf\nborder=3\ngaps=9\n");
        f.set_list("spawn", &["rivet-shell".into(), "foot".into()]);
        f.set_list("spawn", &["rivet-shell".into()]);
        assert_eq!((f.list("spawn"), f.text().lines().count()), (vec!["rivet-shell".to_string()], 4));
    }

    #[test]
    fn includes_are_read_in_place_and_changes_are_seen() {
        let dir = std::env::temp_dir().join(format!("softcore-settings-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let main = dir.join("settings");
        std::fs::write(&main, "gaps=4\ninclude=more.conf\nborder=9\n").unwrap();
        std::fs::write(dir.join("more.conf"), "gaps=20\nborder=1\n").unwrap();
        let c = Config::load_from(&main);
        assert_eq!((c.layout.gaps, c.layout.border, c.files.len()), (20, 9, 2));
        assert!(!c.changed_on_disk());
        let mut f = SettingsFile::load(&main).unwrap();
        f.set("gaps", "30");
        f.save().unwrap();
        assert_eq!(Config::load_from(&main).layout.gaps, 30);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
