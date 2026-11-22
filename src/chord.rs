//! Chords: a binding of several keys pressed one after another (`bind.Mod+G,T=…`).
//!
//! The first keys open the chord; each key after narrows it, until one names an action.
//! While it is open a list shows what can come next (unless `chord_panel=false`), and the
//! next step can be taken by its own key, or by going up and down the list and pressing
//! Enter. Escape leaves, Backspace goes back a step, and any other key leaves too.

use smithay::input::keyboard::Keysym;

use crate::config::{Action, Config, Mods};
use crate::state::State;

/// The keys pressed so far, and which line of the list is picked out.
#[derive(Clone, Debug, PartialEq)]
pub struct Chord {
    pub path: Vec<(Mods, String)>,
    pub selected: usize,
}

/// One thing the next key can be.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub key: (Mods, String),
    /// The key as written in the settings.
    pub shown: String,
    pub label: String,
    /// What it does. None: it leads on to more keys.
    pub action: Option<Action>,
    /// Works, and is left out of the list.
    pub hidden: bool,
}

/// What the list shows.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub title: String,
    /// The key, what it does, and whether more keys follow it.
    pub rows: Vec<(String, String, bool)>,
    pub selected: usize,
    pub vim: bool,
}

/// How a key is written: `Shift+T`.
fn written(mods: &Mods, key: &str) -> String {
    let mut k: Vec<char> = key.chars().collect();
    if let Some(c) = k.first_mut() {
        *c = c.to_ascii_uppercase();
    }
    mods.combo(&k.into_iter().collect::<String>())
}

fn path_written(path: &[(Mods, String)]) -> String {
    path.iter().map(|(m, k)| written(m, k)).collect::<Vec<_>>().join(", ")
}

/// What the keys pressed so far are called.
fn name(c: &Config, path: &[(Mods, String)]) -> Option<String> {
    c.chords.iter().find(|(k, _)| k.as_slice() == path).map(|(_, n)| n.clone()).filter(|n| !n.is_empty())
}

/// What can follow the keys pressed so far, in the order the settings give them.
pub fn steps(c: &Config, path: &[(Mods, String)]) -> Vec<Step> {
    let mut out: Vec<Step> = Vec::new();
    for b in c.binds.iter().filter(|b| !b.then.is_empty()) {
        let keys = b.keys();
        if keys.len() <= path.len() || keys[..path.len()] != *path {
            continue;
        }
        let key = keys[path.len()].clone();
        let leaf = keys.len() == path.len() + 1;
        let step = if leaf {
            Step { shown: written(&key.0, &key.1), label: b.title.clone().filter(|t| !t.is_empty()).unwrap_or_else(|| b.action.label()), action: Some(b.action.clone()), hidden: b.title.as_deref() == Some(""), key }
        } else {
            let to = &keys[..path.len() + 1];
            Step { shown: written(&key.0, &key.1), label: name(c, to).unwrap_or_else(|| "More".into()), action: None, hidden: false, key }
        };
        match out.iter_mut().find(|s| s.key == step.key) {
            // Keys that lead on win over an action on the same key, which they would hide.
            Some(have) if have.action.is_some() && step.action.is_none() => *have = step,
            Some(_) => {}
            None => out.push(step),
        }
    }
    out
}

pub fn view(c: &Config, chord: &Chord) -> View {
    let rows: Vec<(String, String, bool)> = steps(c, &chord.path).into_iter().filter(|s| !s.hidden).map(|s| (s.shown, s.label, s.action.is_none())).collect();
    View { title: name(c, &chord.path).unwrap_or_else(|| path_written(&chord.path)), selected: chord.selected.min(rows.len().saturating_sub(1)), rows, vim: c.chord_vim }
}

/// Whether the key pressed is this one. `lenient`: modifiers still held from the chord's
/// first keys don't count against it, so Mod need not be let go before the next key.
fn is_key(key: &(Mods, String), alt_mod: bool, held: [bool; 4], first: Option<&Mods>, sym: Keysym) -> bool {
    let want = |m: &Mods| [m.ctrl, m.shift, m.alt || (m.modkey && alt_mod), m.logo || (m.modkey && !alt_mod)];
    if crate::input::key_sym(&key.1) != Some(sym) {
        return false;
    }
    let w = want(&key.0);
    match first.map(want) {
        None => held == w,
        Some(f) => (0..4).all(|i| held[i] == w[i] || (held[i] && f[i] && !w[i])),
    }
}

/// What a key press was to the chords.
pub enum Outcome {
    /// Nothing to do with them.
    NotMine,
    /// Theirs, and not for the client. With an action when the chord is complete.
    Taken(Option<Action>),
}

impl State {
    pub fn end_chord(&mut self) {
        if self.chord.take().is_some() {
            self.queue_redraw();
        }
    }

    pub fn chord_key(&mut self, held: [bool; 4], sym: Keysym) -> Outcome {
        let alt_mod = self.mod_is_alt();
        let Some(mut chord) = self.chord.clone() else {
            // The first keys of a chord open it.
            let first = self.config.binds.iter().filter(|b| !b.then.is_empty()).find(|b| is_key(&(b.mods, b.key.clone()), alt_mod, held, None, sym));
            let Some(b) = first else { return Outcome::NotMine };
            self.chord = Some(Chord { path: vec![(b.mods, b.key.clone())], selected: 0 });
            self.queue_redraw();
            return Outcome::Taken(None);
        };
        // Shift and the like on their way down are part of the next key.
        if sym.is_modifier_key() {
            return Outcome::NotMine;
        }
        self.queue_redraw();
        let steps = steps(&self.config, &chord.path);
        let first = chord.path[0].0;
        let found = steps.iter().find(|s| is_key(&s.key, alt_mod, held, None, sym)).or_else(|| steps.iter().find(|s| is_key(&s.key, alt_mod, held, Some(&first), sym))).cloned();
        let shown: Vec<&Step> = steps.iter().filter(|s| !s.hidden).collect();
        let n = shown.len().max(1);
        let at = chord.selected.min(n - 1);
        let vim = self.config.chord_vim;
        let take = |state: &mut State, mut chord: Chord, step: Step| match step.action {
            Some(a) => {
                state.chord = None;
                Outcome::Taken(Some(a))
            }
            None => {
                chord.path.push(step.key);
                chord.selected = 0;
                state.chord = Some(chord);
                Outcome::Taken(None)
            }
        };
        if let Some(step) = found {
            return take(self, chord, step);
        }
        // Keys that are no step of the chord go about its list, when it is showing.
        let listed = self.config.chord_panel;
        match sym {
            Keysym::Down | Keysym::Tab if listed => chord.selected = (at + 1) % n,
            Keysym::j if listed && vim => chord.selected = (at + 1) % n,
            Keysym::Up if listed => chord.selected = (at + n - 1) % n,
            Keysym::k if listed && vim => chord.selected = (at + n - 1) % n,
            Keysym::Return | Keysym::KP_Enter | Keysym::Right if listed => {
                return match shown.get(at) {
                    Some(step) => take(self, chord, (*step).clone()),
                    None => Outcome::Taken(None),
                };
            }
            Keysym::l if listed && vim => {
                return match shown.get(at) {
                    Some(step) => take(self, chord, (*step).clone()),
                    None => Outcome::Taken(None),
                };
            }
            Keysym::BackSpace | Keysym::Left | Keysym::h if sym != Keysym::h || (listed && vim) => {
                if sym == Keysym::Left && !listed {
                    self.chord = None;
                    return Outcome::Taken(None);
                }
                chord.path.pop();
                chord.selected = 0;
                if chord.path.is_empty() {
                    self.chord = None;
                    return Outcome::Taken(None);
                }
            }
            // Escape, and anything that isn't part of it.
            _ => {
                self.chord = None;
                return Outcome::Taken(None);
            }
        }
        self.chord = Some(chord);
        Outcome::Taken(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config::from_text("bind.Mod+G,T=spawn term\nbind.Mod+G,W,F=title=\"Web\" spawn firefox\nbind.Mod+G,W,M=title=\"\" spawn mail\nbind.Mod+G,Shift+T=spawn other\nchord.Mod+G=Go to\nchord.Mod+G,W=Internet\n")
    }

    #[test]
    fn a_chord_lists_what_can_follow() {
        let c = config();
        let g = vec![(Mods { modkey: true, ..Default::default() }, "g".to_string())];
        let v = view(&c, &Chord { path: g.clone(), selected: 0 });
        assert_eq!(v.title, "Go to");
        assert_eq!(v.rows, vec![("T".to_string(), "Run term".to_string(), false), ("W".to_string(), "Internet".to_string(), true), ("Shift+T".to_string(), "Run other".to_string(), false)]);
        // A step with an empty title works and isn't listed.
        let mut w = g;
        w.push((Mods::default(), "w".to_string()));
        let s = steps(&c, &w);
        assert_eq!((s.len(), view(&c, &Chord { path: w, selected: 9 }).rows), (2, vec![("F".to_string(), "Web".to_string(), false)]));
    }

    #[test]
    fn mod_may_stay_down_for_the_next_key() {
        let t = (Mods::default(), "t".to_string());
        let first = Mods { modkey: true, ..Default::default() };
        // Ctrl, Shift, Alt, Super.
        assert!(is_key(&t, false, [false; 4], None, Keysym::t));
        assert!(!is_key(&t, false, [false, false, false, true], None, Keysym::t));
        assert!(is_key(&t, false, [false, false, false, true], Some(&first), Keysym::t));
        assert!(!is_key(&t, false, [true, false, false, true], Some(&first), Keysym::t));
    }
}
