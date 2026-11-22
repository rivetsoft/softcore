//! What each setting is, and the settings read into values.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::{Bind, DEFAULT_BINDS, ModKey, Options, OutputRule, Theme, parse_bind, parse_color, parse_combo, parse_output};

/// The page of a settings window a key belongs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Windows,
    Columns,
    Look,
    Keyboard,
    Pointer,
}

impl Group {
    pub const ALL: [Group; 5] = [Group::Windows, Group::Columns, Group::Look, Group::Keyboard, Group::Pointer];

    pub fn label(self) -> &'static str {
        match self {
            Group::Windows => "Windows",
            Group::Columns => "Columns",
            Group::Look => "Look",
            Group::Keyboard => "Keyboard",
            Group::Pointer => "Pointer",
        }
    }
}

/// What kind of value a key takes, which is also what control edits it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// A whole number: a slider or a stepper.
    Int { min: i32, max: i32 },
    /// A whole number, or empty to follow something else (said in the help).
    OptInt { min: i32, max: i32 },
    /// A number with a fraction: a slider.
    Float { min: f64, max: f64, step: f64 },
    /// `true` or `false`: a switch.
    Bool,
    /// `#rrggbb`: a colour well.
    Color,
    /// `#rrggbb`, or empty to follow the Rivet colours.
    OptColor,
    /// One of a few words: a menu.
    Choice(&'static [&'static str]),
    /// Anything: a text field.
    Text,
    /// Numbers between 0.1 and 1 with commas between: a list of widths.
    Widths,
}

/// One plain setting.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Setting {
    pub key: &'static str,
    pub group: Group,
    pub label: &'static str,
    pub help: &'static str,
    pub kind: Kind,
    /// As it would be written in the file.
    pub default: &'static str,
}

const fn s(key: &'static str, group: Group, label: &'static str, help: &'static str, kind: Kind, default: &'static str) -> Setting {
    Setting { key, group, label, help, kind, default }
}

/// Every plain setting, in the order a window would show them. The defaults here are the
/// defaults: `Config::default()` is built by reading them.
pub const SETTINGS: &[Setting] = &[
    s("gaps", Group::Windows, "Gaps between windows", "In pixels.", Kind::Int { min: 0, max: 200 }, "12"),
    s("gaps_out", Group::Windows, "Gaps at the screen's edges", "In pixels. Empty: the same as between windows.", Kind::OptInt { min: 0, max: 400 }, ""),
    s("border", Group::Windows, "Border width", "In pixels; 0 for none. The border also rounds the window's corners.", Kind::Int { min: 0, max: 40 }, "3"),
    s("radius", Group::Windows, "Corner radius", "In pixels. Empty: Rivet's.", Kind::OptInt { min: 0, max: 100 }, ""),
    s("border_active", Group::Windows, "Focused window's border", "Empty: the Rivet accent.", Kind::OptColor, ""),
    s("border_inactive", Group::Windows, "Other windows' border", "Empty: the Rivet theme's outline.", Kind::OptColor, ""),
    s("inactive_opacity", Group::Windows, "Unfocused windows' opacity", "1 is solid.", Kind::Float { min: 0.1, max: 1.0, step: 0.05 }, "1"),
    s("shadows", Group::Windows, "Shadows", "A soft shadow under each window.", Kind::Bool, "true"),
    s("column_widths", Group::Columns, "Column widths", "The widths the next-width key steps through, as parts of the screen.", Kind::Widths, "0.3333,0.5,0.6667"),
    s("default_width", Group::Columns, "A new column's width", "As a part of the screen.", Kind::Float { min: 0.1, max: 1.0, step: 0.05 }, "0.5"),
    s("theme", Group::Look, "Theme", "For Softcore's own panels and the colours it takes from Rivet.", Kind::Choice(&["system", "light", "dark"]), "system"),
    s("accent", Group::Look, "Accent", "Used when the desktop's colours aren't followed.", Kind::Color, "#3584e4"),
    s("follow_desktop_colors", Group::Look, "Follow the desktop's colours", "The colours Rivet Shell makes from the wallpaper.", Kind::Bool, "true"),
    s("background", Group::Look, "Behind everything", "Where no wallpaper is drawn. Empty: from the Rivet colours.", Kind::OptColor, ""),
    s("mod", Group::Keyboard, "The Mod key", "auto: Super on displays of its own, Alt in a window of another desktop.", Kind::Choice(&["auto", "super", "alt"]), "auto"),
    s("default_binds", Group::Keyboard, "Softcore's own key bindings", "Off: only the bindings in this file.", Kind::Bool, "true"),
    s("xkb_layout", Group::Keyboard, "Layout", "As xkb names it: us, de, us,ru. Empty: the system's.", Kind::Text, ""),
    s("xkb_variant", Group::Keyboard, "Layout variant", "As xkb names it: dvorak, colemak.", Kind::Text, ""),
    s("xkb_options", Group::Keyboard, "Layout options", "As xkb names them: caps:escape,grp:alt_shift_toggle.", Kind::Text, ""),
    s("repeat_delay", Group::Keyboard, "Delay before a held key repeats", "In milliseconds.", Kind::Int { min: 50, max: 5000 }, "600"),
    s("repeat_rate", Group::Keyboard, "Repeats a second", "", Kind::Int { min: 1, max: 200 }, "25"),
    s("tap_to_click", Group::Pointer, "Tap to click", "On touchpads.", Kind::Bool, "true"),
    s("natural_scroll", Group::Pointer, "Natural scrolling", "The page follows the fingers.", Kind::Bool, "false"),
    s("cursor_theme", Group::Pointer, "Cursor theme", "Empty: the desktop's.", Kind::Text, ""),
    s("cursor_size", Group::Pointer, "Cursor size", "In pixels.", Kind::Int { min: 8, max: 256 }, "24"),
];

pub fn setting(key: &str) -> Option<&'static Setting> {
    SETTINGS.iter().find(|s| s.key == key)
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

fn widths(v: &str) -> Option<Vec<f64>> {
    let mut w: Vec<f64> = v.split(',').map(|p| p.trim().parse::<f64>().ok().filter(|p| (0.1..=1.0).contains(p))).collect::<Option<_>>()?;
    w.sort_by(|a, b| a.total_cmp(b));
    (!w.is_empty()).then_some(w)
}

/// Whether a value suits a setting; the message says what would.
pub fn check(s: &Setting, v: &str) -> Result<(), String> {
    let v = v.trim();
    let err = |want: String| Err(format!("{}={v}: {want}", s.key));
    match s.kind {
        Kind::OptInt { .. } | Kind::OptColor if v.is_empty() => Ok(()),
        Kind::Int { min, max } | Kind::OptInt { min, max } => match v.parse::<i32>() {
            Ok(n) if (min..=max).contains(&n) => Ok(()),
            _ => err(format!("a whole number from {min} to {max}")),
        },
        Kind::Float { min, max, .. } => match v.parse::<f64>() {
            Ok(n) if (min..=max).contains(&n) => Ok(()),
            _ => err(format!("a number from {min} to {max}")),
        },
        Kind::Bool => parse_bool(v).map(|_| ()).ok_or(format!("{}={v}: true or false", s.key)),
        Kind::Color | Kind::OptColor => parse_color(v).map(|_| ()).ok_or(format!("{}={v}: a colour like #3584e4", s.key)),
        Kind::Choice(words) if words.contains(&v) => Ok(()),
        Kind::Choice(words) => err(format!("one of {}", words.join(", "))),
        Kind::Text => Ok(()),
        Kind::Widths => widths(v).map(|_| ()).ok_or(format!("{}={v}: widths from 0.1 to 1 with commas between, like 0.3333,0.5,0.6667", s.key)),
    }
}

pub fn config_file() -> PathBuf {
    let var = |k: &str| std::env::var_os(k).map(PathBuf::from).filter(|p| !p.as_os_str().is_empty());
    var("SOFTCORE_CONFIG").unwrap_or_else(|| var("XDG_CONFIG_HOME").or_else(|| var("HOME").map(|h| h.join(".config"))).unwrap_or_else(|| PathBuf::from(".config")).join("softcore/settings"))
}

/// The settings as values.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub layout: Options,
    /// Corner radius of the border; None follows Rivet's.
    pub radius: Option<i32>,
    /// None: the Rivet accent, and the theme's outline.
    pub border_active: Option<[u8; 3]>,
    pub border_inactive: Option<[u8; 3]>,
    pub inactive_opacity: f32,
    pub shadows: bool,
    /// None: from the Rivet colours.
    pub background: Option<[u8; 3]>,
    pub theme: Theme,
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
    /// Each plain setting as written (the default where the files don't say).
    values: BTreeMap<&'static str, String>,
    default_binds: bool,
    /// Bindings from the files, by their keys; None: unbound.
    user_binds: Vec<(String, Option<Bind>)>,
}

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
            theme: Theme::System,
            accent: [0, 0, 0],
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
            values: BTreeMap::new(),
            default_binds: true,
            user_binds: Vec::new(),
        };
        for s in SETTINGS {
            c.set_plain(s, s.default).expect("the defaults are valid");
        }
        c.finish();
        c
    }
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

impl Config {
    /// The settings on disk (the defaults where there are none).
    pub fn load() -> Config {
        Config::load_from(&config_file())
    }

    pub fn load_from(path: &Path) -> Config {
        let mut c = Config::default();
        c.read_file(path, 0);
        c.finish();
        c
    }

    pub fn from_text(text: &str) -> Config {
        let mut c = Config::default();
        c.read_text(text, Path::new("."), 0);
        c.finish();
        c
    }

    /// Whether any file read has changed since (or one that was missing has appeared).
    pub fn changed_on_disk(&self) -> bool {
        self.files.iter().any(|(p, t)| mtime(p) != *t)
    }

    /// A plain setting as it would be written: what a window shows for it.
    pub fn value(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
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
        for line in text.lines() {
            let Some((key, value)) = crate::file::entry(line) else {
                let line = line.trim();
                if !line.is_empty() && !line.starts_with('#') {
                    self.errors.push(format!("{line:?} isn't key=value"));
                }
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

    /// One line of the file.
    fn set(&mut self, key: &str, v: &str) -> Result<(), String> {
        if let Some(s) = setting(key) {
            return self.set_plain(s, v);
        }
        if let Some(combo) = key.strip_prefix("bind.") {
            let (mods, k) = parse_combo(combo)?;
            let id = mods.combo(&k);
            let bind = if v.is_empty() { None } else { Some(parse_bind(combo, v).map_err(|e| format!("{key}={v}: {e}"))?) };
            self.user_binds.retain(|(i, _)| *i != id);
            self.user_binds.push((id, bind));
        } else if let Some(name) = key.strip_prefix("output.") {
            let r = parse_output(name, v)?;
            self.outputs.retain(|o| o.name != r.name);
            self.outputs.push(r);
        } else if let Some(name) = key.strip_prefix("env.") {
            self.env.retain(|(k, _)| k != name);
            self.env.push((name.to_string(), v.to_string()));
        } else if key == "spawn" {
            self.spawn.push(v.to_string());
        }
        // Anything else is a newer Softcore's key, or a typo: skipped, so the rest still loads.
        Ok(())
    }

    fn set_plain(&mut self, s: &'static Setting, v: &str) -> Result<(), String> {
        check(s, v)?;
        let v = v.trim();
        let int = || v.parse::<i32>().ok();
        let boolean = || parse_bool(v).unwrap_or_default();
        let color = || parse_color(v);
        match s.key {
            "gaps" => self.layout.gaps = int().unwrap_or_default(),
            // Kept as written; `finish` makes empty follow `gaps`.
            "gaps_out" => {}
            "border" => self.layout.border = int().unwrap_or_default(),
            "radius" => self.radius = int(),
            "border_active" => self.border_active = color(),
            "border_inactive" => self.border_inactive = color(),
            "background" => self.background = color(),
            "inactive_opacity" => self.inactive_opacity = v.parse().unwrap_or(1.0),
            "shadows" => self.shadows = boolean(),
            "column_widths" => self.layout.presets = widths(v).unwrap_or_default(),
            "default_width" => self.layout.default_width = v.parse().unwrap_or(0.5),
            "theme" => {
                self.theme = match v {
                    "dark" => Theme::Dark,
                    "light" => Theme::Light,
                    _ => Theme::System,
                }
            }
            "accent" => self.accent = color().unwrap_or_default(),
            "follow_desktop_colors" => self.follow_desktop_colors = boolean(),
            "mod" => {
                self.mod_key = match v {
                    "super" => ModKey::Super,
                    "alt" => ModKey::Alt,
                    _ => ModKey::Auto,
                }
            }
            "default_binds" => self.default_binds = boolean(),
            "xkb_layout" => self.xkb_layout = v.to_string(),
            "xkb_variant" => self.xkb_variant = v.to_string(),
            "xkb_options" => self.xkb_options = v.to_string(),
            "repeat_delay" => self.repeat_delay = int().unwrap_or(600),
            "repeat_rate" => self.repeat_rate = int().unwrap_or(25),
            "tap_to_click" => self.tap_to_click = boolean(),
            "natural_scroll" => self.natural_scroll = boolean(),
            "cursor_theme" => self.cursor_theme = v.to_string(),
            "cursor_size" => self.cursor_size = int().unwrap_or(24) as u32,
            other => unreachable!("{other} is in SETTINGS but not read"),
        }
        self.values.insert(s.key, v.to_string());
        Ok(())
    }

    /// What depends on more than one line, once every line is read.
    fn finish(&mut self) {
        self.layout.gaps_out = self.value("gaps_out").and_then(|v| v.parse().ok()).unwrap_or(self.layout.gaps);
        self.binds.clear();
        if self.default_binds {
            for line in DEFAULT_BINDS.lines() {
                let (combo, action) = line.split_once(' ').expect("a default binding is keys and an action");
                self.binds.push(parse_bind(combo, action).expect("the default bindings parse"));
            }
        }
        for (id, bind) in &self.user_binds {
            self.binds.retain(|b| b.mods.combo(&b.key) != *id);
            self.binds.extend(bind.clone());
        }
    }
}
