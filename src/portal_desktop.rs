//! The rest of `softcore portal`: what an app asks of the desktop that only the compositor, or
//! the Rivet desktop around it, can do.
//!
//! - `org.freedesktop.impl.portal.Inhibit`: keep the screen awake (Softcore holds idle off,
//!   as for a window that asks) and keep the computer from suspending (a logind lock).
//! - `org.freedesktop.impl.portal.GlobalShortcuts`: an app's shortcuts that work whichever
//!   window has the keyboard, as Softcore bindings (`app_shortcuts.rs`).
//! - `org.freedesktop.impl.portal.Wallpaper`: a picture as the wallpaper, through Rivet Shell.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};

use serde_json::Value as Json;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedFd, OwnedValue, Value};

use crate::portal::{Dict, ask, owned};

const SUSPEND: u32 = 4;
const IDLE: u32 = 8;

/// What a request is holding on to; dropping it lets go.
type Holds = Arc<Mutex<HashMap<String, Vec<Box<dyn Send>>>>>;

/// A request an app can take back (`org.freedesktop.impl.portal.Request`): letting go of
/// whatever it holds.
struct Request {
    path: String,
    holds: Holds,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Request")]
impl Request {
    async fn close(&self, #[zbus(object_server)] server: &zbus::ObjectServer) {
        self.holds.lock().unwrap().remove(&self.path);
        let _ = server.remove::<Request, _>(self.path.as_str()).await;
    }
}

/// A logind lock, held as long as its file is open.
async fn logind_lock(what: &str, app: &str, why: &str) -> zbus::Result<OwnedFd> {
    let system = zbus::Connection::system().await?;
    let reply = system.call_method(Some("org.freedesktop.login1"), "/org/freedesktop/login1", Some("org.freedesktop.login1.Manager"), "Inhibit", &(what, app, why, "block")).await?;
    reply.body().deserialize::<OwnedFd>()
}

pub struct Inhibit {
    holds: Holds,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Inhibit")]
impl Inhibit {
    async fn inhibit(&self, handle: ObjectPath<'_>, app_id: String, _window: String, flags: u32, options: Dict, #[zbus(object_server)] server: &zbus::ObjectServer) {
        let why = options.get("reason").and_then(|v| String::try_from(v.clone()).ok()).unwrap_or_default();
        let app = if app_id.is_empty() { "An app".to_string() } else { app_id };
        let mut held: Vec<Box<dyn Send>> = Vec::new();
        if flags & IDLE != 0 {
            match blocking::unblock(|| ask(&["inhibit-idle"])).await {
                Ok((_, connection)) => held.push(Box::new(connection)),
                Err(e) => eprintln!("softcore portal: couldn't keep the screen awake: {e}"),
            }
        }
        // Leaving Softcore is always asked for by a person, so of logging out, switching user
        // and suspending, only suspending needs holding off.
        if flags & SUSPEND != 0 {
            match logind_lock("sleep", &app, &why).await {
                Ok(fd) => held.push(Box::new(fd)),
                Err(e) => eprintln!("softcore portal: couldn't keep the computer from suspending: {e}"),
            }
        }
        let path = handle.to_string();
        self.holds.lock().unwrap().insert(path.clone(), held);
        let _ = server.at(&handle, Request { path, holds: self.holds.clone() }).await;
    }
}

impl Inhibit {
    pub fn new() -> Inhibit {
        Inhibit { holds: Holds::default() }
    }
}

/// One app's shortcuts, as the portal knows them.
#[derive(Default)]
struct ShortcutSession {
    app: String,
    /// Each shortcut: its id, what it does, and the keys it has (empty: none).
    shortcuts: Vec<(String, String, String)>,
    /// Softcore's connection; dropping it takes the shortcuts away.
    connection: Option<UnixStream>,
}

type ShortcutSessions = Arc<Mutex<HashMap<String, ShortcutSession>>>;

fn listed(shortcuts: &[(String, String, String)]) -> Dict {
    let list: Vec<(String, HashMap<String, Value<'_>>)> = shortcuts.iter().map(|(id, what, keys)| (id.clone(), HashMap::from([("description".to_string(), Value::from(what.as_str())), ("trigger_description".to_string(), Value::from(keys.as_str()))]))).collect();
    let mut results = Dict::new();
    results.insert("shortcuts".into(), owned(Value::from(list)));
    results
}

fn millis() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Pass Softcore's word that a shortcut went down or up on to the app, until it closes.
fn relay(connection: UnixStream, bus: zbus::Connection, session: String) {
    let Ok(emitter) = SignalEmitter::new(&bus, crate::portal::PATH) else { return };
    for line in BufReader::new(connection).lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<Json>(&line) else { continue };
        let Ok(path) = ObjectPath::try_from(session.as_str()) else { return };
        let none: HashMap<String, Value<'_>> = HashMap::new();
        let sent = if let Some(id) = v["activated"].as_str() {
            zbus::block_on(GlobalShortcuts::activated(&emitter, path, id, millis(), none))
        } else if let Some(id) = v["deactivated"].as_str() {
            zbus::block_on(GlobalShortcuts::deactivated(&emitter, path, id, millis(), none))
        } else {
            Ok(())
        };
        if sent.is_err() {
            return;
        }
    }
}

pub struct GlobalShortcuts {
    sessions: ShortcutSessions,
}

impl GlobalShortcuts {
    pub fn new() -> GlobalShortcuts {
        GlobalShortcuts { sessions: ShortcutSessions::default() }
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.GlobalShortcuts")]
impl GlobalShortcuts {
    async fn create_session(&self, _handle: ObjectPath<'_>, session_handle: ObjectPath<'_>, app_id: String, _options: Dict, #[zbus(object_server)] server: &zbus::ObjectServer) -> (u32, Dict) {
        // An app outside a sandbox has no id of its own to give: bindings name it "app".
        let app = if app_id.is_empty() { "app".to_string() } else { app_id };
        self.sessions.lock().unwrap().insert(session_handle.to_string(), ShortcutSession { app, ..Default::default() });
        let session = ShortcutSessionObject { path: session_handle.to_string(), sessions: self.sessions.clone() };
        if let Err(e) = server.at(&session_handle, session).await {
            eprintln!("softcore portal: couldn't make a session: {e}");
            return (2, Dict::new());
        }
        (0, Dict::new())
    }

    async fn bind_shortcuts(&self, _handle: ObjectPath<'_>, session_handle: ObjectPath<'_>, shortcuts: Vec<(String, HashMap<String, OwnedValue>)>, _parent_window: String, _options: Dict, #[zbus(connection)] bus: &zbus::Connection) -> (u32, Dict) {
        let Some(app) = self.sessions.lock().unwrap().get_mut(session_handle.as_str()).map(|s| {
            s.connection = None;
            s.app.clone()
        }) else {
            return (2, Dict::new());
        };
        let text = |d: &HashMap<String, OwnedValue>, k: &str| d.get(k).and_then(|v| String::try_from(v.clone()).ok()).unwrap_or_default();
        let wanted: Vec<String> = shortcuts.iter().map(|(id, d)| format!("{id}={}", text(d, "preferred_trigger"))).collect();
        let what: HashMap<String, String> = shortcuts.iter().map(|(id, d)| (id.clone(), text(d, "description"))).collect();
        let asked = blocking::unblock(move || {
            let mut words = vec!["app-shortcuts", app.as_str()];
            words.extend(wanted.iter().map(String::as_str));
            ask(&words)
        })
        .await;
        let (answer, connection) = match asked {
            Ok(a) => a,
            Err(e) => {
                eprintln!("softcore portal: no shortcuts: {e}");
                return (2, Dict::new());
            }
        };
        let bound: Vec<(String, String, String)> = answer.as_array().into_iter().flatten().map(|s| {
            let id = s["id"].as_str().unwrap_or_default().to_string();
            (id.clone(), what.get(&id).cloned().unwrap_or_default(), s["trigger"].as_str().unwrap_or_default().to_string())
        }).collect();
        let Ok(listening) = connection.try_clone() else { return (2, Dict::new()) };
        let (bus, session) = (bus.clone(), session_handle.to_string());
        std::thread::spawn(move || relay(listening, bus, session));
        let results = listed(&bound);
        if let Some(s) = self.sessions.lock().unwrap().get_mut(session_handle.as_str()) {
            s.shortcuts = bound;
            s.connection = Some(connection);
        }
        (0, results)
    }

    async fn list_shortcuts(&self, _handle: ObjectPath<'_>, session_handle: ObjectPath<'_>) -> (u32, Dict) {
        match self.sessions.lock().unwrap().get(session_handle.as_str()) {
            Some(s) => (0, listed(&s.shortcuts)),
            None => (2, Dict::new()),
        }
    }

    #[zbus(signal)]
    async fn activated(emitter: &SignalEmitter<'_>, session_handle: ObjectPath<'_>, shortcut_id: &str, timestamp: u64, options: HashMap<String, Value<'_>>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn deactivated(emitter: &SignalEmitter<'_>, session_handle: ObjectPath<'_>, shortcut_id: &str, timestamp: u64, options: HashMap<String, Value<'_>>) -> zbus::Result<()>;

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
}

struct ShortcutSessionObject {
    path: String,
    sessions: ShortcutSessions,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Session")]
impl ShortcutSessionObject {
    async fn close(&self, #[zbus(object_server)] server: &zbus::ObjectServer, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        // Dropping the session drops its connection, and Softcore forgets the shortcuts.
        self.sessions.lock().unwrap().remove(&self.path);
        let _ = Self::closed(&emitter).await;
        let _ = server.remove::<ShortcutSessionObject, _>(self.path.as_str()).await;
    }

    #[zbus(signal)]
    async fn closed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
}

/// A `file://` URI as a path.
fn uri_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(b) = rest.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

pub struct Wallpaper;

#[zbus::interface(name = "org.freedesktop.impl.portal.Wallpaper")]
impl Wallpaper {
    #[zbus(name = "SetWallpaperURI")]
    async fn set_wallpaper_uri(&self, _handle: ObjectPath<'_>, _app_id: String, _parent_window: String, uri: String, options: Dict) -> u32 {
        let set_on = options.get("set-on").and_then(|v| String::try_from(v.clone()).ok()).unwrap_or_else(|| "both".into());
        // Rivet's lock screen is a program of its own choosing, with no picture to set.
        if set_on == "lockscreen" {
            return 2;
        }
        let Some(path) = uri_path(&uri) else { return 2 };
        // Whether the app may is xdg-desktop-portal's to ask, before this is called. A preview
        // (`show-preview`) there is none of: the desktop shows the picture at once.
        let name = std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let set = blocking::unblock(move || {
            // A picture handed over in a passing file is kept, so the wallpaper outlives it.
            let keep = path.starts_with("/tmp/") || path.starts_with("/run/") || std::env::var("XDG_RUNTIME_DIR").is_ok_and(|r| path.starts_with(&r));
            let path = if keep {
                let dir = rivet_xdg::places::data_home().join("softcore/wallpapers");
                let to = dir.join(&name);
                std::fs::create_dir_all(&dir).and_then(|_| std::fs::copy(&path, &to)).map(|_| to.display().to_string()).unwrap_or(path)
            } else {
                path
            };
            std::process::Command::new("rivet-shell").args(["msg", "wallpaper", &path]).status().is_ok_and(|s| s.success())
        })
        .await;
        if set {
            0
        } else {
            eprintln!("softcore portal: couldn't set the wallpaper: Rivet Shell isn't running");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_uri_is_a_path() {
        assert_eq!(uri_path("file:///home/me/Pictures/Big%20Sur.jpg").as_deref(), Some("/home/me/Pictures/Big Sur.jpg"));
        assert_eq!(uri_path("https://example.com/a.jpg"), None);
    }
}
