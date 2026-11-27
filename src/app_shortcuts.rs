//! Apps' own shortcuts that work whichever window has the keyboard (the global shortcuts
//! portal). `softcore portal` asks for an app's shortcuts with `app-shortcuts APP ID=KEYS…`
//! and keeps the connection; each shortcut the app suggested keys for gets them, unless
//! Softcore's bindings or another app already use them. A key bound to `app-shortcut APP ID`
//! in the settings reaches the same shortcut, which is how one with no keys of its own gets
//! some. Softcore then tells the asker `{"activated": ID}` when the keys go down and
//! `{"deactivated": ID}` when they come up; closing the connection takes the shortcuts away.

use std::io::Write;
use std::os::unix::net::UnixStream;

use serde_json::json;
use smithay::input::keyboard::Keysym;

use crate::config::Mods;
use crate::state::State;

pub struct App {
    id: String,
    /// Each shortcut, with the keys it has, if any.
    shortcuts: Vec<(String, Option<(Mods, String)>)>,
    stream: UnixStream,
    _hangup: crate::state::Hangup,
}

#[derive(Default)]
pub struct AppShortcuts {
    apps: Vec<App>,
    /// Keys down that started a shortcut, to say when they come up: the key's code, the app, the shortcut.
    held: Vec<(u32, String, String)>,
}

/// Whether two keys are the same once Mod is read as the key it is here (Super, or Alt).
fn same(a: &(Mods, String), b: &(Mods, String), alt_mod: bool) -> bool {
    let plain = |m: &Mods| (m.ctrl, m.shift, m.alt || (m.modkey && alt_mod), m.logo || (m.modkey && !alt_mod));
    plain(&a.0) == plain(&b.0) && a.1.eq_ignore_ascii_case(&b.1)
}

/// The shortcuts portal's way of writing keys (`CTRL+SHIFT+a`, `LOGO+F9`) as Softcore's.
fn trigger(text: &str) -> Option<(Mods, String)> {
    let (mods, key) = crate::config::parse_combo(text).ok()?;
    crate::input::key_sym(&key)?;
    Some((mods, key.to_lowercase()))
}

/// An app asks for its shortcuts. Answered with the keys each got (empty: none).
pub fn listen(state: &mut State, app: &str, wanted: &[&str], mut stream: UnixStream) {
    let mut taken: Vec<(Mods, String)> = state.config.binds.iter().filter(|b| b.then.is_empty()).map(|b| (b.mods, b.key.to_lowercase())).collect();
    taken.extend(state.app_shortcuts.apps.iter().flat_map(|a| a.shortcuts.iter().filter_map(|(_, k)| k.clone())));
    let mut shortcuts = Vec::new();
    for w in wanted {
        let (id, keys) = w.rsplit_once('=').unwrap_or((w, ""));
        let keys = trigger(keys).filter(|k| !taken.iter().any(|t| same(t, k, state.mod_is_alt())));
        if let Some(k) = &keys {
            taken.push(k.clone());
        }
        shortcuts.push((id.to_string(), keys));
    }
    let answer: Vec<_> = shortcuts.iter().map(|(id, k)| json!({"id": id, "trigger": k.as_ref().map(|(m, key)| m.combo(key)).unwrap_or_default()})).collect();
    if writeln!(stream, "{}", json!({"ok": answer})).is_err() {
        return;
    }
    let _ = stream.set_nonblocking(true);
    let hangup = crate::state::Hangup::new(&state.handle, &stream);
    state.app_shortcuts.apps.push(App { id: app.to_string(), shortcuts, stream, _hangup: hangup });
}

fn tell(state: &mut State, app: &str, what: &str, id: &str) {
    for a in state.app_shortcuts.apps.iter_mut().filter(|a| a.id == app && a.shortcuts.iter().any(|(s, _)| s == id)) {
        let _ = a.stream.set_nonblocking(false);
        let _ = writeln!(a.stream, "{}", json!({what: id}));
        let _ = a.stream.set_nonblocking(true);
    }
}

/// A key went down that no binding of Softcore's took. True when it was an app's shortcut.
pub fn pressed(state: &mut State, code: u32, held: (bool, bool, bool, bool), sym: Keysym) -> bool {
    let alt_mod = state.mod_is_alt();
    let found = state.app_shortcuts.apps.iter().find_map(|a| a.shortcuts.iter().find(|(_, k)| k.as_ref().is_some_and(|(m, key)| crate::input::combo_matches(m, key, alt_mod, held, sym))).map(|(id, _)| (a.id.clone(), id.clone())));
    let Some((app, id)) = found else { return false };
    tell(state, &app, "activated", &id);
    state.app_shortcuts.held.push((code, app, id));
    true
}

/// A key Softcore took came up: the shortcut it started, if any, is over.
pub fn released(state: &mut State, code: u32) {
    while let Some(i) = state.app_shortcuts.held.iter().position(|(c, _, _)| *c == code) {
        let (_, app, id) = state.app_shortcuts.held.remove(i);
        tell(state, &app, "deactivated", &id);
    }
}

/// A binding in the settings names an app's shortcut: it happens at once.
pub fn fire(state: &mut State, app: &str, id: &str) {
    tell(state, app, "activated", id);
    tell(state, app, "deactivated", id);
}

/// Once round the loop: an app whose connection closed takes its shortcuts with it.
pub fn refresh(state: &mut State) {
    state.app_shortcuts.apps.retain_mut(|a| match std::io::Read::read(&mut a.stream, &mut [0u8; 64]) {
        Ok(0) => false,
        Err(e) => e.kind() == std::io::ErrorKind::WouldBlock,
        Ok(_) => true,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mod_is_the_key_it_stands_for() {
        let (super_return, mod_return) = (trigger("LOGO+Return").unwrap(), (Mods { modkey: true, ..Default::default() }, "return".to_string()));
        assert!(same(&super_return, &mod_return, false));
        assert!(!same(&super_return, &mod_return, true));
    }

    #[test]
    fn the_portals_keys_are_read_as_softcores() {
        let (m, k) = trigger("CTRL+SHIFT+a").unwrap();
        assert!(m.ctrl && m.shift && !m.alt && !m.logo && k == "a");
        let (m, k) = trigger("LOGO+F9").unwrap();
        assert!(m.logo && k == "f9");
        assert!(trigger("CTRL+nosuchkey").is_none());
        assert!(trigger("").is_none());
    }
}
