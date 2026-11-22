//! Softcore's settings: `~/.config/softcore/settings`, `key=value` lines like every Rivet app's,
//! hand-editable, with unknown keys skipped so a newer file still loads.
//!
//! A key that can be given more than once (`bind`, `spawn`, `env`, `output`, `include`) adds
//! to the ones before it. `include=FILE` reads another file in place (Rivet Shell writes its
//! window look to one), and later lines win over earlier ones. The files are read again when
//! they change.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::layout::{Dir, Options};

/// What a key binding or `softcore msg action` does. The names are niri's where the action
/// is the same, so bindings carry over.
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
    MaximizeColumn,
    FullscreenWindow,
    CenterColumn,
    ToggleFloating,
    SwitchFloatingFocus,
    FocusWorkspace(usize),
    FocusWorkspaceBy(i32),
    MoveToWorkspace { idx: usize, column: bool },
    MoveToWorkspaceBy { by: i32, column: bool },
    FocusMonitor(i32),
    MoveColumnToMonitor(i32),
    ShowHotkeyOverlay,
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
            "set-column-width" => Action::SetColumnWidth(parse_width(rest)?),
            "maximize-column" => Action::MaximizeColumn,
            "fullscreen-window" => Action::FullscreenWindow,
            "center-column" => Action::CenterColumn,
            "toggle-window-floating" => Action::ToggleFloating,
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
            Action::MaximizeColumn => "Column as wide as the screen".into(),
            Action::FullscreenWindow => "Full screen".into(),
            Action::CenterColumn => "Centre the column".into(),
            Action::ToggleFloating => "Float the window, or put it back".into(),
            Action::SwitchFloatingFocus => "Between floating windows and columns".into(),
            Action::FocusWorkspace(n) => format!("Workspace {}", n + 1),
            Action::FocusWorkspaceBy(b) => format!("Workspace {}", if *b > 0 { "below" } else { "above" }),
            Action::MoveToWorkspace { idx, .. } => format!("Move to workspace {}", idx + 1),
            Action::MoveToWorkspaceBy { by, .. } => format!("Move to the workspace {}", if *by > 0 { "below" } else { "above" }),
            Action::FocusMonitor(b) => format!("{} display", if *b > 0 { "Next" } else { "Previous" }),
            Action::MoveColumnToMonitor(b) => format!("Move to the {} display", if *b > 0 { "next" } else { "previous" }),
            Action::ShowHotkeyOverlay => "Show these keys".into(),
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

fn parse_width(t: &str) -> Result<WidthChange, String> {
    let bad = || format!("set-column-width needs a width like 50%, +10% or -10%, not {t:?}");
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

fn parse_bind(value: &str) -> Result<Bind, String> {
    let (combo, action) = value.trim().split_once(char::is_whitespace).ok_or(format!("bind={value} needs a key and an action"))?;
    let (mods, key) = parse_combo(combo)?;
    if crate::input::key_sym(&key).is_none() {
        return Err(format!("bind={value}: there is no key called {key:?}"));
    }
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

/// A display's settings: `output=NAME mode=2560x1440@59.951 scale=1 position=0,0`, or
/// `output=NAME off`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct OutputRule {
    pub name: String,
    pub off: bool,
    /// Width, height and, when given, the refresh rate in mHz.
    pub mode: Option<(i32, i32, Option<i32>)>,
    pub scale: Option<f64>,
    pub position: Option<(i32, i32)>,
}

fn parse_output(value: &str) -> Result<OutputRule, String> {
    let mut parts = value.split_whitespace();
    let mut r = OutputRule { name: parts.next().ok_or("output needs a display's name")?.to_string(), ..Default::default() };
    for p in parts {
        let bad = || format!("output={value}: couldn't read {p:?}");
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
            _ => return Err(bad()),
        }
    }
    Ok(r)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub layout: Options,
    /// Corner radius of the border; None follows Rivet's (`rivet_look::theme::r_md`).
    pub radius: Option<i32>,
    /// None: the Rivet accent, and a quiet grey that suits light or dark.
    pub border_active: Option<[u8; 3]>,
    pub border_inactive: Option<[u8; 3]>,
    pub inactive_opacity: f32,
    pub shadows: bool,
    /// Behind everything, where no wallpaper is drawn. None: from the Rivet colours.
    pub background: Option<[u8; 3]>,
    pub theme: rivet_look::theme::Mode,
    pub accent: [u8; 3],
    pub follow_desktop_colors: bool,
    pub mod_key: ModKey,
    pub repeat_delay: i32,
    pub repeat_rate: i32,
    pub xkb_layout: String,
    pub xkb_variant: String,
    pub xkb_options: String,
    pub natural_scroll: bool,
    pub tap_to_click: bool,
    pub cursor_theme: String,
    pub cursor_size: u32,
    pub spawn: Vec<String>,
    pub env: Vec<(String, String)>,
    pub binds: Vec<Bind>,
    pub outputs: Vec<OutputRule>,
    /// Lines that couldn't be used, for the panel that says so.
    pub errors: Vec<String>,
    /// Every file read, and when each was last changed.
    pub files: Vec<(PathBuf, Option<SystemTime>)>,
}

/// The bindings Softcore starts with. `default_binds=false` drops them; a `bind` on the same
/// keys replaces one.
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
Mod+F maximize-column
Mod+Shift+F fullscreen-window
Mod+C center-column
Mod+V toggle-window-floating
Mod+Shift+V switch-focus-between-floating-and-tiling
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

impl Default for Config {
    fn default() -> Self {
        let mut c = Config {
            layout: Options::default(),
            radius: None,
            border_active: None,
            border_inactive: None,
            inactive_opacity: 1.0,
            shadows: true,
            background: None,
            theme: rivet_look::theme::Mode::System,
            accent: rivet_look::theme::ACCENTS[0].0,
            follow_desktop_colors: true,
            mod_key: ModKey::Auto,
            repeat_delay: 600,
            repeat_rate: 25,
            xkb_layout: String::new(),
            xkb_variant: String::new(),
            xkb_options: String::new(),
            natural_scroll: false,
            tap_to_click: true,
            cursor_theme: String::new(),
            cursor_size: 24,
            spawn: Vec::new(),
            env: Vec::new(),
            binds: Vec::new(),
            outputs: Vec::new(),
            errors: Vec::new(),
            files: Vec::new(),
        };
        for line in DEFAULT_BINDS.lines() {
            c.binds.push(parse_bind(line).expect("the default bindings parse"));
        }
        c
    }
}

pub fn config_file() -> PathBuf {
    std::env::var_os("SOFTCORE_CONFIG").map(PathBuf::from).filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| rivet_xdg::places::config_home().join("softcore/settings"))
}

pub fn parse_color(v: &str) -> Option<[u8; 3]> {
    let h = v.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(h, 16).ok()?;
    Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

impl Config {
    /// The settings on disk (the defaults where there are none).
    pub fn load() -> Config {
        let mut c = Config::default();
        c.read_file(&config_file(), 0);
        c
    }

    #[cfg(test)]
    pub fn from_text(text: &str) -> Config {
        let mut c = Config::default();
        c.read_text(text, Path::new("."), 0);
        c
    }

    /// Whether any file read has changed since (or one that was missing has appeared).
    pub fn changed_on_disk(&self) -> bool {
        self.files.iter().any(|(p, t)| mtime(p) != *t)
    }

    fn read_file(&mut self, path: &Path, depth: u32) {
        self.files.push((path.to_path_buf(), mtime(path)));
        match std::fs::read_to_string(path) {
            Ok(text) => self.read_text(&text, path.parent().unwrap_or(Path::new(".")), depth),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => self.errors.push(format!("{}: {e}", path.display())),
        }
    }

    fn read_text(&mut self, text: &str, dir: &Path, depth: u32) {
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=').map(|(k, v)| (k.trim(), v.trim())) else {
                self.errors.push(format!("{line:?} isn't key=value"));
                continue;
            };
            if key == "include" {
                if depth < 8 {
                    self.read_file(&dir.join(value), depth + 1);
                }
                continue;
            }
            if let Err(e) = self.set(key, value) {
                self.errors.push(e);
            }
        }
    }

    fn set(&mut self, key: &str, v: &str) -> Result<(), String> {
        let bad = || format!("{key}={v}: couldn't read the value");
        let int = |lo: i32, hi: i32| v.parse::<i32>().ok().filter(|n| (lo..=hi).contains(n)).ok_or_else(bad);
        let boolean = || parse_bool(v).ok_or_else(bad);
        // Empty: back to following Rivet's colours.
        let color = || if v.is_empty() { Ok(None) } else { parse_color(v).map(Some).ok_or_else(bad) };
        match key {
            "gaps" => {
                // The edges follow unless they were set apart (gaps_out comes after gaps).
                self.layout.gaps = int(0, 200)?;
                self.layout.gaps_out = self.layout.gaps;
            }
            "gaps_out" => self.layout.gaps_out = int(0, 400)?,
            "border" => self.layout.border = int(0, 40)?,
            "radius" => self.radius = if v.is_empty() { None } else { Some(int(0, 100)?) },
            "border_active" => self.border_active = color()?,
            "border_inactive" => self.border_inactive = color()?,
            "background" => self.background = color()?,
            "inactive_opacity" => self.inactive_opacity = v.parse::<f32>().ok().filter(|o| (0.1..=1.0).contains(o)).ok_or_else(bad)?,
            "shadows" => self.shadows = boolean()?,
            "column_widths" => {
                let mut w: Vec<f64> = v.split(',').map(|p| p.trim().parse::<f64>().ok().filter(|p| (0.1..=1.0).contains(p))).collect::<Option<_>>().ok_or_else(bad)?;
                w.sort_by(|a, b| a.total_cmp(b));
                if w.is_empty() {
                    return Err(bad());
                }
                self.layout.presets = w;
            }
            "default_width" => self.layout.default_width = v.parse::<f64>().ok().filter(|p| (0.1..=1.0).contains(p)).ok_or_else(bad)?,
            "theme" => {
                self.theme = match v {
                    "dark" => rivet_look::theme::Mode::Dark,
                    "light" => rivet_look::theme::Mode::Light,
                    "system" => rivet_look::theme::Mode::System,
                    _ => return Err(bad()),
                }
            }
            "accent" => self.accent = parse_color(v).ok_or_else(bad)?,
            "follow_desktop_colors" => self.follow_desktop_colors = boolean()?,
            "mod" => {
                self.mod_key = match v.to_ascii_lowercase().as_str() {
                    "auto" => ModKey::Auto,
                    "super" => ModKey::Super,
                    "alt" => ModKey::Alt,
                    _ => return Err(bad()),
                }
            }
            "repeat_delay" => self.repeat_delay = int(50, 5000)?,
            "repeat_rate" => self.repeat_rate = int(1, 200)?,
            "xkb_layout" => self.xkb_layout = v.to_string(),
            "xkb_variant" => self.xkb_variant = v.to_string(),
            "xkb_options" => self.xkb_options = v.to_string(),
            "natural_scroll" => self.natural_scroll = boolean()?,
            "tap_to_click" => self.tap_to_click = boolean()?,
            "cursor_theme" => self.cursor_theme = v.to_string(),
            "cursor_size" => self.cursor_size = int(8, 256)? as u32,
            "spawn" => self.spawn.push(v.to_string()),
            "env" => {
                let (k, val) = v.split_once('=').ok_or_else(bad)?;
                self.env.push((k.trim().to_string(), val.trim().to_string()));
            }
            "default_binds" => {
                if !boolean()? {
                    self.binds.clear();
                }
            }
            "bind" => {
                let b = parse_bind(v)?;
                self.binds.retain(|o| (o.mods, &o.key) != (b.mods, &b.key));
                self.binds.push(b);
            }
            "unbind" => {
                let (mods, k) = parse_combo(v)?;
                self.binds.retain(|o| (o.mods, &o.key) != (mods, &k));
            }
            "output" => {
                let r = parse_output(v)?;
                self.outputs.retain(|o| o.name != r.name);
                self.outputs.push(r);
            }
            // A newer Softcore's key, or a typo: skipped, so the rest still loads.
            _ => {}
        }
        Ok(())
    }
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_look_and_skips_what_it_doesnt_know() {
        let c = Config::from_text("# look\ngaps=8\ngaps_out=14\nborder=2\nradius=10\nborder_active=#e66100\nfuture_key=1\ncolumn_widths=0.5, 0.25\n");
        assert_eq!((c.layout.gaps, c.layout.gaps_out, c.layout.border, c.radius), (8, 14, 2, Some(10)));
        assert_eq!(c.border_active, Some([0xe6, 0x61, 0x00]));
        assert_eq!(c.layout.presets, vec![0.25, 0.5]);
        assert!(c.errors.is_empty());
    }

    #[test]
    fn bad_lines_are_reported_and_the_rest_still_loads() {
        let c = Config::from_text("gaps=many\nborder=4\nbind=Mod+T fly\nnonsense\n");
        assert_eq!(c.layout.border, 4);
        assert_eq!(c.errors.len(), 3);
    }

    #[test]
    fn binds_replace_and_unbind() {
        let c = Config::from_text("bind=Mod+Q spawn foot\nunbind=Mod+E\nbind=Mod+Shift+T allow-when-locked spawn true\n");
        let q: Vec<&Bind> = c.binds.iter().filter(|b| b.key == "q").collect();
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].action, Action::Spawn("foot".into()));
        assert!(!c.binds.iter().any(|b| b.key == "e" && !b.mods.shift));
        assert!(c.binds.last().unwrap().locked && c.binds.last().unwrap().mods.shift);
        assert!(Config::from_text("default_binds=false\n").binds.is_empty());
    }

    #[test]
    fn actions_and_outputs_parse() {
        assert_eq!(Action::parse("focus-workspace 3"), Ok(Action::FocusWorkspace(2)));
        assert_eq!(Action::parse("set-column-width -10%"), Ok(Action::SetColumnWidth(WidthChange::By(-0.1))));
        assert_eq!(Action::parse("quit --skip-confirmation"), Ok(Action::Quit { confirm: false }));
        assert!(Action::parse("focus-workspace").is_err());
        let o = parse_output("DP-5 mode=2560x1440@59.951 scale=1.5 position=2560,0").unwrap();
        assert_eq!((o.mode, o.scale, o.position), (Some((2560, 1440, Some(59951))), Some(1.5), Some((2560, 0))));
        assert!(parse_output("eDP-1 off").unwrap().off);
    }

    #[test]
    fn includes_are_read_in_place() {
        let dir = std::env::temp_dir().join(format!("softcore-config-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("settings"), "gaps=4\ninclude=rivet-shell.conf\nborder=9\n").unwrap();
        std::fs::write(dir.join("rivet-shell.conf"), "gaps=20\nborder=1\n").unwrap();
        let mut c = Config::default();
        c.read_file(&dir.join("settings"), 0);
        assert_eq!((c.layout.gaps, c.layout.border, c.files.len()), (20, 9, 2));
        assert!(!c.changed_on_disk());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
