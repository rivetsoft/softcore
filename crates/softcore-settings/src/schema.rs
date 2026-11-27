//! What each setting is, and the settings read into values.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::{Bind, LayerRule, ScratchpadRule, WindowRule, WorkspaceRule, Scroll, TitleBars, WindowLayout, DEFAULT_BINDS, ModKey, Options, OutputRule, Theme, parse_bind, parse_color, parse_output};

/// The page of a settings window a key belongs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Windows,
    Columns,
    Look,
    Motion,
    Keyboard,
    Pointer,
}

impl Group {
    pub const ALL: [Group; 6] = [Group::Windows, Group::Columns, Group::Look, Group::Motion, Group::Keyboard, Group::Pointer];

    pub fn label(self) -> &'static str {
        match self {
            Group::Windows => "Windows",
            Group::Columns => "Columns",
            Group::Look => "Look",
            Group::Motion => "Motion",
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
    s("border_active_to", Group::Windows, "Focused window's border runs to", "A second colour makes the border a gradient from the first to this one. Empty: one colour.", Kind::OptColor, ""),
    s("border_inactive_to", Group::Windows, "Other windows' border runs to", "Empty: one colour.", Kind::OptColor, ""),
    s("border_angle", Group::Windows, "Border gradient angle", "The way a gradient runs, in degrees: 0 is upwards, 90 to the right, 180 downwards.", Kind::Int { min: 0, max: 360 }, "135"),
    s("inactive_opacity", Group::Windows, "Unfocused windows' opacity", "1 is solid.", Kind::Float { min: 0.1, max: 1.0, step: 0.05 }, "1"),
    s("shadows", Group::Windows, "Shadows", "A soft shadow under each window.", Kind::Bool, "true"),
    s("shadow_size", Group::Windows, "Shadow size", "How far a shadow reaches past its window, in pixels. Larger is also softer.", Kind::Int { min: 1, max: 120 }, "28"),
    s("shadow_offset", Group::Windows, "Shadow offset", "How far below its window a shadow sits, in pixels.", Kind::Int { min: -60, max: 60 }, "7"),
    s("shadow_opacity", Group::Windows, "Shadow strength", "0 is none, 1 is solid at the window's edge.", Kind::Float { min: 0.0, max: 1.0, step: 0.05 }, "0.4"),
    s("shadow_color", Group::Windows, "Shadow colour", "", Kind::Color, "#000000"),
    s("title_bars", Group::Windows, "Title bars", "A bar over a window with its name and buttons to close, maximize and float it; drag it to move the window. off: none, the border is all. floating: on floating windows. on: on every window.", Kind::Choice(&["off", "floating", "on"]), "off"),
    s("title_bar_height", Group::Windows, "Title bar height", "In pixels.", Kind::Int { min: 16, max: 64 }, "30"),
    s("title_bar_active", Group::Windows, "Focused window's title bar", "Empty: from the Rivet theme.", Kind::OptColor, ""),
    s("title_bar_inactive", Group::Windows, "Other windows' title bars", "Empty: from the Rivet theme.", Kind::OptColor, ""),
    s("title_bar_text", Group::Windows, "Title bar text", "Empty: from the Rivet theme.", Kind::OptColor, ""),
    s("title_bar_buttons", Group::Windows, "Title bar buttons", "Which side of the bar the buttons sit on.", Kind::Choice(&["right", "left"]), "right"),
    s("snap", Group::Windows, "Floating windows snap", "A floating window moved by the pointer lines up with the screen's edges and other floating windows, and fills half or a quarter of the screen when the pointer reaches an edge or a corner, or all of it at the top.", Kind::Bool, "true"),
    s("xwayland", Group::Windows, "X11 programs", "Programs written for X11 open through xwayland-satellite, started when the first one asks. Read when Softcore starts.", Kind::Bool, "true"),
    s("blur", Group::Windows, "Blur behind see-through windows", "What shows through a window that isn't solid is blurred.", Kind::Bool, "true"),
    s("blur_strength", Group::Windows, "Blur strength", "1 is slight, 10 is heavy.", Kind::Int { min: 1, max: 10 }, "5"),
    s("blur_layers", Group::Windows, "Shell surfaces to blur behind", "Their names, with commas between; all of a named surface's rectangle is blurred behind, so name only ones that fill theirs. A surface that asks for blur itself gets it without being named.", Kind::Text, ""),
    s("layout", Group::Columns, "Windows are placed", "scrolling: in columns on a strip that scrolls. floating: every window floats where it is put, at the size it is given. A display or a workspace can say otherwise.", Kind::Choice(&["scrolling", "floating"]), "scrolling"),
    s("scroll", Group::Columns, "Windows scroll", "horizontal: columns side by side, scrolling left and right. vertical: rows one under the other, scrolling up and down. auto: vertical on a display taller than it is wide.", Kind::Choice(&["horizontal", "vertical", "auto"]), "horizontal"),
    s("column_widths", Group::Columns, "Column widths", "The widths the next-width key steps through, as parts of the screen.", Kind::Widths, "0.3333,0.5,0.6667"),
    s("default_width", Group::Columns, "A new column's width", "As a part of the screen.", Kind::Float { min: 0.1, max: 1.0, step: 0.05 }, "0.5"),
    s("center_short_columns", Group::Columns, "Centre short columns", "A column made shorter than the screen sits in the middle of its height. Off: at the top.", Kind::Bool, "true"),
    s("overview_zoom", Group::Columns, "Overview size", "How small the workspaces are drawn in the overview, as a part of their usual size.", Kind::Float { min: 0.2, max: 0.9, step: 0.05 }, "0.5"),
    s("tab_bar", Group::Columns, "Tab strip thickness", "The strip over a tabbed column, a piece for each of its windows.", Kind::Int { min: 2, max: 40 }, "8"),
    s("scratchpad_size", Group::Columns, "Scratchpad window size", "How much of the screen a window shown from the scratchpad takes, each way.", Kind::Float { min: 0.2, max: 1.0, step: 0.05 }, "0.7"),
    s("theme", Group::Look, "Theme", "For Softcore's own panels, the colours it takes from Rivet, and the dark or light apps are told.", Kind::Choice(&["system", "light", "dark"]), "system"),
    s("accent", Group::Look, "Accent", "Used when the desktop's colours aren't followed.", Kind::Color, "#3584e4"),
    s("follow_desktop_colors", Group::Look, "Follow the desktop's colours", "The colours Rivet Shell makes from the wallpaper.", Kind::Bool, "true"),
    s("background", Group::Look, "Behind everything", "Where no wallpaper is drawn. Empty: from the Rivet colours.", Kind::OptColor, ""),
    s("animations", Group::Motion, "Animations", "Windows fade in and out and slide to their places. Off: everything is where it belongs at once.", Kind::Bool, "true"),
    s("animation_speed", Group::Motion, "Animation speed", "1 is usual, 2 twice as fast.", Kind::Float { min: 0.25, max: 4.0, step: 0.25 }, "1"),
    s("mod", Group::Keyboard, "The Mod key", "auto: Super on displays of its own, Alt in a window of another desktop.", Kind::Choice(&["auto", "super", "alt"]), "auto"),
    s("chord_panel", Group::Keyboard, "Show what can follow a chord's first keys", "A binding of several keys, one after another (bind.Mod+G,T=…), opens a list of what the next key can be. Off: the keys work the same with nothing shown.", Kind::Bool, "true"),
    s("chord_vim", Group::Keyboard, "H, J, K and L go about a chord's list", "Down, up, back a step and on, as in Vim, beside the arrow keys. A step of the chord on one of those letters still wins.", Kind::Bool, "false"),
    s("default_binds", Group::Keyboard, "Softcore's own key bindings", "Off: only the bindings in this file.", Kind::Bool, "true"),
    s("xkb_layout", Group::Keyboard, "Layout", "As xkb names it: us, de, us,ru. Empty: the system's.", Kind::Text, ""),
    s("xkb_variant", Group::Keyboard, "Layout variant", "As xkb names it: dvorak, colemak.", Kind::Text, ""),
    s("xkb_options", Group::Keyboard, "Layout options", "As xkb names them: caps:escape,grp:alt_shift_toggle.", Kind::Text, ""),
    s("repeat_delay", Group::Keyboard, "Delay before a held key repeats", "In milliseconds.", Kind::Int { min: 50, max: 5000 }, "600"),
    s("repeat_rate", Group::Keyboard, "Repeats a second", "", Kind::Int { min: 1, max: 200 }, "25"),
    s("tap_to_click", Group::Pointer, "Tap to click", "On touchpads.", Kind::Bool, "true"),
    s("natural_scroll", Group::Pointer, "Natural scrolling", "The page follows the fingers.", Kind::Bool, "false"),
    s("screenshot_dir", Group::Look, "Where screenshots go", "A folder. Empty: Screenshots in your Pictures.", Kind::Text, ""),
    s("screenshot_edit", Group::Look, "Marks screenshots up", "The program E opens a screenshot in, as it is chosen. The picture's file is added to the end. Empty: none.", Kind::Text, "rivet-snip --replace"),
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
    /// The colour each border runs to as a gradient; None: one colour.
    pub border_active_to: Option<[u8; 3]>,
    pub border_inactive_to: Option<[u8; 3]>,
    /// Degrees: 0 upwards, 90 to the right.
    pub border_angle: i32,
    pub inactive_opacity: f32,
    pub shadows: bool,
    pub xwayland: bool,
    /// How far a shadow reaches, how far below the window it sits, and how strong it is.
    pub shadow_size: i32,
    pub shadow_offset: i32,
    pub shadow_opacity: f32,
    pub overview_zoom: f64,
    pub shadow_color: [u8; 3],
    pub blur: bool,
    pub blur_strength: i32,
    /// None: from the Rivet colours.
    pub title_bar_active: Option<[u8; 3]>,
    pub title_bar_inactive: Option<[u8; 3]>,
    pub title_bar_text: Option<[u8; 3]>,
    /// The buttons on the bar's left, else its right.
    pub title_bar_buttons_left: bool,
    pub snap: bool,
    /// Shell surfaces (by the name each gives itself) to blur behind.
    pub blur_layers: Vec<String>,
    pub animations: bool,
    /// 1 is usual. `layout.animation_speed` is this, or 0 with animations off.
    pub animation_speed: f64,
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
    pub screenshot_dir: String,
    pub screenshot_edit: String,
    pub cursor_size: u32,
    pub spawn: Vec<String>,
    pub env: Vec<(String, String)>,
    pub binds: Vec<Bind>,
    pub outputs: Vec<OutputRule>,
    /// How particular windows, workspaces, scratchpads and shell surfaces are treated, in
    /// the order their lines stand.
    /// What the keys that begin a chord are called in its panel: `chord.Mod+G=Go to`.
    pub chords: Vec<(Vec<(crate::Mods, String)>, String)>,
    pub chord_panel: bool,
    pub chord_vim: bool,
    pub rules: Vec<WindowRule>,
    pub workspaces: Vec<WorkspaceRule>,
    pub scratchpads: Vec<ScratchpadRule>,
    pub layers: Vec<LayerRule>,
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
            border_active_to: None,
            border_inactive_to: None,
            border_angle: 135,
            inactive_opacity: 1.0,
            shadows: true,
            xwayland: true,
            shadow_size: 28,
            shadow_offset: 7,
            shadow_opacity: 0.4,
            overview_zoom: 0.5,
            shadow_color: [0, 0, 0],
            blur: true,
            blur_strength: 5,
            title_bar_active: None,
            title_bar_inactive: None,
            title_bar_text: None,
            title_bar_buttons_left: false,
            snap: true,
            blur_layers: Vec::new(),
            animations: true,
            animation_speed: 1.0,
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
            screenshot_dir: String::new(),
            screenshot_edit: "rivet-snip --replace".into(),
            cursor_size: 24,
            spawn: Vec::new(),
            env: Vec::new(),
            binds: Vec::new(),
            outputs: Vec::new(),
            chords: Vec::new(),
            chord_panel: true,
            chord_vim: false,
            rules: Vec::new(),
            workspaces: Vec::new(),
            scratchpads: Vec::new(),
            layers: Vec::new(),
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

    /// What the lines that fit a window give it between them, later lines over earlier.
    pub fn window_rule(&self, app_id: &str, title: &str) -> WindowRule {
        let mut out = WindowRule::default();
        for r in self.rules.iter().filter(|r| r.matches.fits(app_id, title)) {
            out.take(r);
        }
        // A scratchpad's own line says which windows are its.
        if let Some(s) = self.scratchpads.iter().rev().find(|s| !s.matches.is_empty() && s.matches.fits(app_id, title)) {
            out.scratchpad = Some(s.name.clone());
        }
        out
    }

    /// What the lines that fit a shell surface's name give it between them.
    pub fn layer_rule(&self, namespace: &str) -> LayerRule {
        let mut out = LayerRule { namespace: namespace.to_string(), ..Default::default() };
        if self.blur_layers.iter().any(|n| n == namespace) {
            out.blur = Some(true);
        }
        for r in self.layers.iter().filter(|r| crate::rules::glob(&r.namespace, namespace)) {
            out.opacity = r.opacity.or(out.opacity);
            out.blur = r.blur.or(out.blur);
            out.shadow = r.shadow.or(out.shadow);
        }
        out
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
            let id = crate::keys_id(&crate::parse_keys(combo)?);
            let bind = if v.is_empty() { None } else { Some(parse_bind(combo, v).map_err(|e| format!("{key}={v}: {e}"))?) };
            self.user_binds.retain(|(i, _)| *i != id);
            self.user_binds.push((id, bind));
        } else if let Some(name) = key.strip_prefix("output.") {
            let r = parse_output(name, v)?;
            self.outputs.retain(|o| o.name != r.name);
            self.outputs.push(r);
        } else if let Some(keys) = key.strip_prefix("chord.") {
            let keys = crate::parse_keys(keys)?;
            self.chords.retain(|(k, _)| *k != keys);
            self.chords.push((keys, v.to_string()));
        } else if let Some(name) = key.strip_prefix("rule.") {
            let r = crate::rules::parse_window_rule(name, v)?;
            self.rules.retain(|o| o.name != r.name);
            self.rules.push(r);
        } else if let Some(name) = key.strip_prefix("workspace.") {
            let r = crate::rules::parse_workspace_rule(name, v)?;
            self.workspaces.retain(|o| o.name != r.name);
            self.workspaces.push(r);
        } else if let Some(name) = key.strip_prefix("scratchpad.") {
            let r = crate::rules::parse_scratchpad_rule(name, v)?;
            self.scratchpads.retain(|o| o.name != r.name);
            self.scratchpads.push(r);
        } else if let Some(name) = key.strip_prefix("layer.") {
            let r = crate::rules::parse_layer_rule(name, v)?;
            self.layers.retain(|o| o.namespace != r.namespace);
            self.layers.push(r);
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
            "border_active_to" => self.border_active_to = color(),
            "border_inactive_to" => self.border_inactive_to = color(),
            "border_angle" => self.border_angle = int().unwrap_or(135),
            "background" => self.background = color(),
            "inactive_opacity" => self.inactive_opacity = v.parse().unwrap_or(1.0),
            "shadows" => self.shadows = boolean(),
            "xwayland" => self.xwayland = boolean(),
            "shadow_size" => self.shadow_size = int().unwrap_or(28),
            "shadow_offset" => self.shadow_offset = int().unwrap_or(7),
            "overview_zoom" => self.overview_zoom = v.parse::<f64>().unwrap_or(0.5).clamp(0.2, 0.9),
            "shadow_opacity" => self.shadow_opacity = v.parse().unwrap_or(0.4),
            "shadow_color" => self.shadow_color = color().unwrap_or_default(),
            "blur" => self.blur = boolean(),
            "blur_strength" => self.blur_strength = int().unwrap_or(5),
            "blur_layers" => self.blur_layers = v.split(',').map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).collect(),
            "animations" => self.animations = boolean(),
            "animation_speed" => self.animation_speed = v.parse().unwrap_or(1.0),
            "column_widths" => self.layout.presets = widths(v).unwrap_or_default(),
            "scroll" => self.layout.scroll = Scroll::parse(v).unwrap_or_default(),
            "layout" => self.layout.layout = WindowLayout::parse(v).unwrap_or_default(),
            "title_bars" => {
                self.layout.title_bars = match v {
                    "floating" => TitleBars::Floating,
                    "on" => TitleBars::On,
                    _ => TitleBars::Off,
                }
            }
            "title_bar_height" => self.layout.title_bar = int().unwrap_or(30).clamp(16, 64),
            "title_bar_active" => self.title_bar_active = color(),
            "title_bar_inactive" => self.title_bar_inactive = color(),
            "title_bar_text" => self.title_bar_text = color(),
            "title_bar_buttons" => self.title_bar_buttons_left = v == "left",
            "snap" => self.snap = boolean(),
            "tab_bar" => self.layout.tab_bar = int().unwrap_or(8).clamp(2, 40),
            "center_short_columns" => self.layout.center_short_columns = boolean(),
            "scratchpad_size" => self.layout.scratchpad_size = v.parse().unwrap_or(0.7),
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
            "chord_panel" => self.chord_panel = boolean(),
            "chord_vim" => self.chord_vim = boolean(),
            "xkb_layout" => self.xkb_layout = v.to_string(),
            "xkb_variant" => self.xkb_variant = v.to_string(),
            "xkb_options" => self.xkb_options = v.to_string(),
            "repeat_delay" => self.repeat_delay = int().unwrap_or(600),
            "repeat_rate" => self.repeat_rate = int().unwrap_or(25),
            "tap_to_click" => self.tap_to_click = boolean(),
            "natural_scroll" => self.natural_scroll = boolean(),
            "cursor_theme" => self.cursor_theme = v.to_string(),
            "screenshot_dir" => self.screenshot_dir = v.to_string(),
            "screenshot_edit" => self.screenshot_edit = v.to_string(),
            "cursor_size" => self.cursor_size = int().unwrap_or(24) as u32,
            other => unreachable!("{other} is in SETTINGS but not read"),
        }
        self.values.insert(s.key, v.to_string());
        Ok(())
    }

    /// What depends on more than one line, once every line is read.
    fn finish(&mut self) {
        self.layout.gaps_out = self.value("gaps_out").and_then(|v| v.parse().ok()).unwrap_or(self.layout.gaps);
        self.layout.animation_speed = if self.animations { self.animation_speed } else { 0.0 };
        self.binds.clear();
        if self.default_binds {
            for line in DEFAULT_BINDS.lines() {
                let (combo, action) = line.split_once(' ').expect("a default binding is keys and an action");
                self.binds.push(parse_bind(combo, action).expect("the default bindings parse"));
            }
        }
        for (id, bind) in &self.user_binds {
            self.binds.retain(|b| crate::keys_id(&b.keys()) != *id);
            self.binds.extend(bind.clone());
        }
    }
}
