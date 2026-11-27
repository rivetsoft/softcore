//! Lines that say how particular things are treated: windows by their program and title,
//! workspaces and scratchpads with names, and the shell's surfaces by the name each gives
//! itself.
//!
//! ```text
//! rule.video=app-id=mpv floating opacity=1
//! rule.chat=app-id=*discord* title="* - Discord" workspace=chat
//! workspace.chat=output=DP-2 scroll=vertical layout=floating
//! scratchpad.term=app-id=scratch-term command="rivet-terminal --class scratch-term"
//! layer.rivet-panel=opacity=0.9 blur=true
//! ```
//!
//! What follows the `=` is words with spaces between; one that holds a space goes in
//! quotes. A word that names a program or a title may use `*` for anything.

use crate::{Scroll, WindowLayout};

/// A workspace, by where it is in its display's stack (from 0) or by its name.
#[derive(Clone, Debug, PartialEq)]
pub enum WorkspaceRef {
    Index(usize),
    Name(String),
}

impl WorkspaceRef {
    /// `3` is the third workspace; anything else is a name.
    pub fn parse(v: &str) -> Option<WorkspaceRef> {
        let v = v.trim();
        match v.parse::<usize>() {
            Ok(0) => None,
            Ok(n) => Some(WorkspaceRef::Index(n - 1)),
            Err(_) if v.is_empty() => None,
            Err(_) => Some(WorkspaceRef::Name(v.to_string())),
        }
    }

    pub fn label(&self) -> String {
        match self {
            WorkspaceRef::Index(n) => (n + 1).to_string(),
            WorkspaceRef::Name(n) => n.clone(),
        }
    }
}

/// Which windows a line is about. Every part given has to fit; none given fits them all.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Match {
    pub app_id: Option<String>,
    pub title: Option<String>,
}

impl Match {
    pub fn fits(&self, app_id: &str, title: &str) -> bool {
        self.app_id.as_deref().is_none_or(|p| glob(p, app_id)) && self.title.as_deref().is_none_or(|p| glob(p, title))
    }

    pub fn is_empty(&self) -> bool {
        self.app_id.is_none() && self.title.is_none()
    }
}

/// Whether text fits a pattern in which `*` stands for anything, capitals aside.
pub fn glob(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.to_lowercase().chars().collect(), text.to_lowercase().chars().collect());
    let (mut pi, mut ti) = (0, 0);
    // Where the last `*` was, and how much of the text it has taken so far.
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if pi < p.len() && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == '*')
}

/// What a window is given. Each part is None where no line says.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct WindowRule {
    pub name: String,
    pub matches: Match,
    // As it opens.
    pub floating: Option<bool>,
    pub workspace: Option<WorkspaceRef>,
    pub output: Option<String>,
    /// Part of the screen's width, 0–1.
    pub width: Option<f64>,
    pub fullscreen: Option<bool>,
    pub maximized: Option<bool>,
    /// Put away in this scratchpad as it opens.
    pub scratchpad: Option<String>,
    /// False: it opens without taking the focus.
    pub focus: Option<bool>,
    // For as long as it fits.
    /// Whether it has the focus or not.
    pub opacity: Option<f32>,
    pub border: Option<bool>,
    pub shadow: Option<bool>,
    pub blur: Option<bool>,
    /// Softcore's title bar over it, whatever the title_bars setting says.
    pub title_bar: Option<bool>,
}

impl WindowRule {
    /// Lay another line's choices over these.
    pub fn take(&mut self, o: &WindowRule) {
        macro_rules! over {
            ($($f:ident),*) => { $( if o.$f.is_some() { self.$f = o.$f.clone(); } )* };
        }
        over!(floating, workspace, output, width, fullscreen, maximized, scratchpad, focus, opacity, border, shadow, blur, title_bar);
    }
}

/// A workspace that is always there under its name.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct WorkspaceRule {
    pub name: String,
    /// The display it is made on. None: the first.
    pub output: Option<String>,
    pub scroll: Option<Scroll>,
    pub layout: Option<WindowLayout>,
}

/// A scratchpad with a name: its own key shows it and puts it away.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ScratchpadRule {
    pub name: String,
    /// Windows that go into it as they open.
    pub matches: Match,
    /// Run when it is asked for and holds nothing.
    pub command: Option<String>,
    /// How much of the screen its window takes, each way. None: as `scratchpad_size` says.
    pub size: Option<f64>,
}

/// A surface of the shell, by the name it gives itself (`*` for anything).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct LayerRule {
    pub namespace: String,
    pub opacity: Option<f32>,
    pub blur: Option<bool>,
    pub shadow: Option<bool>,
}

/// Words with spaces between; quotes keep a space inside a word.
pub fn words(v: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    let mut any = false;
    for c in v.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    out.push(std::mem::take(&mut word));
                    any = false;
                }
            }
            c => {
                word.push(c);
                any = true;
            }
        }
    }
    if quoted {
        return Err("a quote is never closed".into());
    }
    if any {
        out.push(word);
    }
    Ok(out)
}

fn flag(v: Option<&str>) -> Option<bool> {
    match v {
        None => Some(true),
        Some(v) => match v.to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "1" => Some(true),
            "false" | "no" | "off" | "0" => Some(false),
            _ => None,
        },
    }
}

/// `0.9`, or `90%`.
fn part(v: &str, min: f64, max: f64) -> Option<f64> {
    let n = match v.strip_suffix('%') {
        Some(p) => p.parse::<f64>().ok()? / 100.0,
        None => v.parse::<f64>().ok()?,
    };
    (min..=max).contains(&n).then_some(n)
}

/// A word as its name and what follows its `=`.
fn split(w: &str) -> (&str, Option<&str>) {
    match w.split_once('=') {
        Some((k, v)) => (k, Some(v)),
        None => (w, None),
    }
}

pub fn parse_window_rule(name: &str, value: &str) -> Result<WindowRule, String> {
    let mut r = WindowRule { name: name.to_string(), ..Default::default() };
    for w in words(value).map_err(|e| format!("rule.{name}: {e}"))? {
        let (k, v) = split(&w);
        let bad = || format!("rule.{name}: couldn't read {w:?}");
        let on = || flag(v).ok_or_else(bad);
        let text = || v.filter(|v| !v.is_empty()).map(str::to_string).ok_or_else(bad);
        match k {
            "app-id" => r.matches.app_id = Some(text()?),
            "title" => r.matches.title = Some(text()?),
            "floating" => r.floating = Some(on()?),
            "tiled" => r.floating = Some(!on()?),
            "workspace" => r.workspace = Some(v.and_then(WorkspaceRef::parse).ok_or_else(bad)?),
            "output" => r.output = Some(text()?),
            "width" => r.width = Some(v.and_then(|v| part(v, 0.05, 1.0)).ok_or_else(bad)?),
            "fullscreen" => r.fullscreen = Some(on()?),
            "maximized" => r.maximized = Some(on()?),
            "scratchpad" => r.scratchpad = Some(text()?),
            "focus" => r.focus = Some(on()?),
            "opacity" => r.opacity = Some(v.and_then(|v| part(v, 0.0, 1.0)).ok_or_else(bad)? as f32),
            "border" => r.border = Some(on()?),
            "shadow" => r.shadow = Some(on()?),
            "blur" => r.blur = Some(on()?),
            "title-bar" => r.title_bar = Some(on()?),
            _ => return Err(bad()),
        }
    }
    Ok(r)
}

pub fn parse_workspace_rule(name: &str, value: &str) -> Result<WorkspaceRule, String> {
    if name.is_empty() || name.parse::<usize>().is_ok() {
        return Err(format!("workspace.{name}: a workspace's name can't be empty or a number"));
    }
    let mut r = WorkspaceRule { name: name.to_string(), ..Default::default() };
    for w in words(value).map_err(|e| format!("workspace.{name}: {e}"))? {
        let (k, v) = split(&w);
        let bad = || format!("workspace.{name}: couldn't read {w:?}");
        match (k, v) {
            ("output", Some(v)) if !v.is_empty() => r.output = Some(v.to_string()),
            ("scroll", Some(v)) => r.scroll = Some(Scroll::parse(v).ok_or_else(bad)?),
            ("layout", Some(v)) => r.layout = Some(WindowLayout::parse(v).ok_or_else(bad)?),
            _ => return Err(bad()),
        }
    }
    Ok(r)
}

pub fn parse_scratchpad_rule(name: &str, value: &str) -> Result<ScratchpadRule, String> {
    if name.is_empty() {
        return Err("scratchpad.: a scratchpad needs a name".into());
    }
    let mut r = ScratchpadRule { name: name.to_string(), ..Default::default() };
    for w in words(value).map_err(|e| format!("scratchpad.{name}: {e}"))? {
        let (k, v) = split(&w);
        let bad = || format!("scratchpad.{name}: couldn't read {w:?}");
        let text = || v.filter(|v| !v.is_empty()).map(str::to_string).ok_or_else(bad);
        match k {
            "app-id" => r.matches.app_id = Some(text()?),
            "title" => r.matches.title = Some(text()?),
            "command" => r.command = Some(text()?),
            "size" => r.size = Some(v.and_then(|v| part(v, 0.1, 1.0)).ok_or_else(bad)?),
            _ => return Err(bad()),
        }
    }
    Ok(r)
}

pub fn parse_layer_rule(namespace: &str, value: &str) -> Result<LayerRule, String> {
    let mut r = LayerRule { namespace: namespace.to_string(), ..Default::default() };
    for w in words(value).map_err(|e| format!("layer.{namespace}: {e}"))? {
        let (k, v) = split(&w);
        let bad = || format!("layer.{namespace}: couldn't read {w:?}");
        match k {
            "opacity" => r.opacity = Some(v.and_then(|v| part(v, 0.0, 1.0)).ok_or_else(bad)? as f32),
            "blur" => r.blur = Some(flag(v).ok_or_else(bad)?),
            "shadow" => r.shadow = Some(flag(v).ok_or_else(bad)?),
            _ => return Err(bad()),
        }
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_star_stands_for_anything() {
        assert!(glob("firefox", "Firefox") && !glob("firefox", "firefox-esr"));
        assert!(glob("*fox*", "org.mozilla.firefox") && glob("* - Discord", "general - Discord"));
        assert!(glob("*", "") && glob("a*b*c", "aXXbYYc") && !glob("a*b*c", "aXXbYY"));
        assert!(!glob("", "x") && glob("", ""));
    }

    #[test]
    fn rules_read_their_words() {
        let r = parse_window_rule("chat", "app-id=*discord* title=\"* - Discord\" workspace=chat floating opacity=90% width=40% border=off title-bar=on").unwrap();
        assert_eq!((r.matches.app_id.as_deref(), r.matches.title.as_deref()), (Some("*discord*"), Some("* - Discord")));
        assert_eq!((r.floating, r.workspace.clone(), r.opacity, r.width, r.border), (Some(true), Some(WorkspaceRef::Name("chat".into())), Some(0.9), Some(0.4), Some(false)));
        assert_eq!(r.title_bar, Some(true));
        assert!(r.matches.fits("com.discordapp.Discord", "general - Discord") && !r.matches.fits("mpv", "general - Discord"));
        assert_eq!(parse_window_rule("n", "workspace=3 tiled").map(|r| (r.workspace, r.floating)), Ok((Some(WorkspaceRef::Index(2)), Some(false))));
        assert!(parse_window_rule("n", "opacity=2").is_err() && parse_window_rule("n", "sideways").is_err() && parse_window_rule("n", "title=\"x").is_err());
        let s = parse_scratchpad_rule("term", "app-id=scratch command=\"foot --app-id scratch\" size=0.6").unwrap();
        assert_eq!((s.command.as_deref(), s.size), (Some("foot --app-id scratch"), Some(0.6)));
        assert_eq!(parse_workspace_rule("chat", "output=DP-2 scroll=vertical").map(|w| (w.output, w.scroll)), Ok((Some("DP-2".into()), Some(Scroll::Vertical))));
        assert!(parse_workspace_rule("3", "").is_err());
        assert_eq!(parse_layer_rule("rivet-*", "opacity=0.8 blur").map(|l| (l.opacity, l.blur)), Ok((Some(0.8), Some(true))));
    }

    #[test]
    fn later_lines_lie_over_earlier_ones() {
        let mut a = parse_window_rule("a", "floating opacity=0.5").unwrap();
        a.take(&parse_window_rule("b", "opacity=0.8 width=0.3").unwrap());
        assert_eq!((a.floating, a.opacity, a.width), (Some(true), Some(0.8), Some(0.3)));
    }
}
