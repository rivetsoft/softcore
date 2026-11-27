//! `softcore portal`: Softcore's backend for xdg-desktop-portal, the service apps that aren't
//! Rivet's talk to.
//!
//! - `org.freedesktop.impl.portal.ScreenCast`: an app in a call asks to share the screen; this
//!   asks Softcore to show its picker and then to start a stream, and hands the stream's
//!   PipeWire number back.
//! - `org.freedesktop.impl.portal.Screenshot`: a picture of the screen, chosen in Softcore's
//!   own screenshot chooser or taken at once, and a color picked from the screen.
//! - `org.freedesktop.impl.portal.Settings`: dark or light, the accent and the contrast
//!   (`appearance.rs`).
//!
//! It holds a connection to Softcore for every stream it started. Softcore stops a stream
//! when its connection closes, so an app closing its session, or this service going away,
//! always ends the sharing.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{Value as Json, json};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

const MONITOR: u32 = 1;
const WINDOW: u32 = 2;
/// The pointer drawn into the picture.
const CURSOR_EMBEDDED: u32 = 2;

pub type Dict = HashMap<String, OwnedValue>;

#[derive(Default)]
struct Session {
    types: u32,
    cursor: bool,
    /// One connection for each stream; dropping them stops the streams.
    streams: Vec<UnixStream>,
}

type Sessions = Arc<Mutex<HashMap<String, Session>>>;

/// Softcore's socket: the one this was started with, else the newest there is.
fn socket() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("SOFTCORE_SOCKET").filter(|p| !p.is_empty()) {
        return Some(p.into());
    }
    let dir = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
    let mut found: Vec<_> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("softcore.") && n.to_string_lossy().ends_with(".sock"))).collect();
    found.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    found.pop()
}

/// Ask Softcore something and wait for its one-line answer. The connection comes back too:
/// for a stream, keeping it open keeps the stream.
pub fn ask(words: &[&str]) -> Result<(Json, UnixStream), String> {
    let path = socket().ok_or("Softcore isn't running")?;
    let mut stream = UnixStream::connect(&path).map_err(|e| format!("couldn't reach Softcore: {e}"))?;
    writeln!(stream, "{}", json!(words)).map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line).map_err(|e| e.to_string())?;
    let v: Json = serde_json::from_str(&line).map_err(|_| "Softcore went away".to_string())?;
    match v.get("error").and_then(Json::as_str) {
        Some(e) => Err(e.to_string()),
        None => Ok((v["ok"].clone(), stream)),
    }
}

/// "org.mozilla.firefox" as a name to show: "Firefox".
fn app_name(app_id: &str) -> String {
    let last = app_id.rsplit('.').next().unwrap_or(app_id);
    let mut c = last.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

struct Stream {
    node: u32,
    size: (i32, i32),
    source_type: u32,
    connection: UnixStream,
}

/// Have the person choose, then start the stream. `Ok(None)`: they chose nothing.
fn pick_and_start(types: u32, cursor: bool, app: String) -> Result<Option<Stream>, String> {
    let mut kinds = Vec::new();
    if types & MONITOR != 0 {
        kinds.push("output");
    }
    if types & WINDOW != 0 {
        kinds.push("window");
    }
    let chosen = match ask(&["pick-cast", "--types", &kinds.join(","), "--app", &app]) {
        Ok((c, _)) => c,
        Err(e) if e == "nothing was chosen" => return Ok(None),
        Err(e) => return Err(e),
    };
    let kind = chosen["kind"].as_str().unwrap_or("output").to_string();
    let target = match &chosen["target"] {
        Json::String(s) => s.clone(),
        other => other.to_string(),
    };
    let mut words = vec!["cast", "start", kind.as_str(), target.as_str()];
    if cursor {
        words.push("--cursor");
    }
    let (started, connection) = ask(&words)?;
    let n = |k: &str| started[k].as_i64().unwrap_or(0);
    Ok(Some(Stream { node: n("node_id") as u32, size: (n("width") as i32, n("height") as i32), source_type: if kind == "output" { MONITOR } else { WINDOW }, connection }))
}

/// A path as a `file://` URI.
fn file_uri(path: &str) -> String {
    let mut uri = String::from("file://");
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

struct Screenshot;

#[zbus::interface(name = "org.freedesktop.impl.portal.Screenshot")]
impl Screenshot {
    async fn screenshot(&self, _handle: ObjectPath<'_>, _app_id: String, _parent_window: String, options: Dict) -> (u32, Dict) {
        let interactive = options.get("interactive").and_then(|v| bool::try_from(v).ok()).unwrap_or(false);
        // Saved where the person's own screenshots go, so a picture an app took is theirs too.
        let file = crate::shot::default_file(&softcore_settings::Config::load().screenshot_dir).display().to_string();
        let asked = blocking::unblock(move || {
            let words: Vec<&str> = if interactive { vec!["pick-screenshot", &file] } else { vec!["pick-screenshot", "--now", &file] };
            ask(&words).map(|(v, _)| v)
        })
        .await;
        match asked {
            Ok(Json::String(path)) => {
                let mut results = Dict::new();
                results.insert("uri".into(), owned(Value::from(file_uri(&path))));
                (0, results)
            }
            Err(e) if e == "nothing was chosen" => (1, Dict::new()),
            other => {
                eprintln!("softcore portal: no screenshot: {other:?}");
                (2, Dict::new())
            }
        }
    }

    async fn pick_color(&self, _handle: ObjectPath<'_>, _app_id: String, _parent_window: String, _options: Dict) -> (u32, Dict) {
        match blocking::unblock(|| ask(&["pick-color"]).map(|(v, _)| v)).await {
            Ok(Json::Array(rgb)) if rgb.len() == 3 => {
                let c = |i: usize| rgb[i].as_f64().unwrap_or(0.0) / 255.0;
                let mut results = Dict::new();
                results.insert("color".into(), owned(Value::from((c(0), c(1), c(2)))));
                (0, results)
            }
            Err(e) if e == "nothing was chosen" => (1, Dict::new()),
            other => {
                eprintln!("softcore portal: no color: {other:?}");
                (2, Dict::new())
            }
        }
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }
}

pub fn owned(v: Value<'_>) -> OwnedValue {
    v.try_to_owned().expect("a value with no file descriptors can be kept")
}

struct ScreenCast {
    sessions: Sessions,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.ScreenCast")]
impl ScreenCast {
    async fn create_session(&self, _handle: ObjectPath<'_>, session_handle: ObjectPath<'_>, _app_id: String, _options: Dict, #[zbus(object_server)] server: &zbus::ObjectServer) -> (u32, Dict) {
        self.sessions.lock().unwrap().insert(session_handle.to_string(), Session { types: MONITOR | WINDOW, ..Default::default() });
        let session = SessionObject { path: session_handle.to_string(), sessions: self.sessions.clone() };
        if let Err(e) = server.at(&session_handle, session).await {
            eprintln!("softcore portal: couldn't make a session: {e}");
            return (2, Dict::new());
        }
        (0, Dict::new())
    }

    async fn select_sources(&self, _handle: ObjectPath<'_>, session_handle: ObjectPath<'_>, _app_id: String, options: Dict) -> (u32, Dict) {
        let number = |k: &str| options.get(k).and_then(|v| u32::try_from(v).ok());
        let mut sessions = self.sessions.lock().unwrap();
        let Some(s) = sessions.get_mut(session_handle.as_str()) else { return (2, Dict::new()) };
        if let Some(t) = number("types").filter(|t| t & (MONITOR | WINDOW) != 0) {
            s.types = t;
        }
        s.cursor = number("cursor_mode") == Some(CURSOR_EMBEDDED);
        (0, Dict::new())
    }

    async fn start(&self, _handle: ObjectPath<'_>, session_handle: ObjectPath<'_>, app_id: String, _parent_window: String, _options: Dict) -> (u32, Dict) {
        let Some((types, cursor)) = self.sessions.lock().unwrap().get(session_handle.as_str()).map(|s| (s.types, s.cursor)) else { return (2, Dict::new()) };
        // The choosing takes as long as the person takes: off this thread, so closing the
        // session and other apps' requests are still answered meanwhile.
        let stream = match blocking::unblock(move || pick_and_start(types, cursor, app_name(&app_id))).await {
            Ok(Some(s)) => s,
            Ok(None) => return (1, Dict::new()),
            Err(e) => {
                eprintln!("softcore portal: {e}");
                return (2, Dict::new());
            }
        };
        let mut props: HashMap<String, Value<'_>> = HashMap::new();
        props.insert("size".into(), Value::from(stream.size));
        props.insert("source_type".into(), Value::from(stream.source_type));
        let streams: Vec<(u32, HashMap<String, Value<'_>>)> = vec![(stream.node, props)];
        let mut results = Dict::new();
        results.insert("streams".into(), owned(Value::from(streams)));
        match self.sessions.lock().unwrap().get_mut(session_handle.as_str()) {
            Some(s) => s.streams.push(stream.connection),
            // Closed while the person was choosing: the stream ends with its connection.
            None => return (1, Dict::new()),
        }
        (0, results)
    }

    #[zbus(property)]
    fn available_source_types(&self) -> u32 {
        MONITOR | WINDOW
    }

    #[zbus(property)]
    fn available_cursor_modes(&self) -> u32 {
        1 | CURSOR_EMBEDDED
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        4
    }
}

struct SessionObject {
    path: String,
    sessions: Sessions,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Session")]
impl SessionObject {
    async fn close(&self, #[zbus(object_server)] server: &zbus::ObjectServer, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        // Dropping the session drops its connections, and Softcore stops the streams.
        self.sessions.lock().unwrap().remove(&self.path);
        let _ = Self::closed(&emitter).await;
        let _ = server.remove::<SessionObject, _>(self.path.as_str()).await;
    }

    #[zbus(signal)]
    async fn closed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
}

pub const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.softcore";
/// Where every backend interface lives, as xdg-desktop-portal expects.
pub const PATH: &str = "/org/freedesktop/portal/desktop";

pub fn run() -> i32 {
    // Started by Softcore for its session: go when it goes.
    if std::env::var_os("SOFTCORE_SOCKET").is_some() {
        unsafe {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
        }
    }
    let made = zbus::blocking::connection::Builder::session()
        .and_then(|b| b.name(BUS_NAME))
        .and_then(|b| b.serve_at(PATH, ScreenCast { sessions: Sessions::default() }))
        .and_then(|b| b.serve_at(PATH, Screenshot))
        .and_then(|b| b.serve_at(PATH, crate::appearance::Settings))
        .and_then(|b| b.serve_at(PATH, crate::portal_desktop::Inhibit::new()))
        .and_then(|b| b.serve_at(PATH, crate::portal_desktop::GlobalShortcuts::new()))
        .and_then(|b| b.serve_at(PATH, crate::portal_desktop::Wallpaper))
        .and_then(|b| b.build());
    match made {
        Ok(connection) => {
            crate::appearance::watch(connection.into_inner());
            0
        }
        Err(e) => {
            eprintln!("softcore portal: couldn't start on the session bus: {e}");
            1
        }
    }
}
