//! Talking to a running Softcore: `softcore msg …` and anything else that opens its socket
//! (`$SOFTCORE_SOCKET`).
//!
//! A request is one line, a JSON array of words (`["action","focus-workspace","2"]`); the
//! answer is one line, `{"ok": …}` or `{"error": "…"}`. `["event-stream"]` keeps the
//! connection open and sends a line whenever workspaces, windows or the focus change. What is
//! sent for workspaces, windows and outputs has the same fields as niri's, so a shell that
//! reads one reads the other.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, LoopHandle, Mode, PostAction};
use smithay::reexports::wayland_server::Resource;

use crate::config::Action;
use crate::state::State;

pub struct Ipc {
    path: PathBuf,
    subscribers: Vec<UnixStream>,
    /// What listeners were last told, to say only what changed.
    last: (String, String, Option<u32>),
}

impl Ipc {
    pub fn start(handle: &LoopHandle<'static, State>, wayland_socket: &str) -> Ipc {
        let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let path = dir.join(format!("softcore.{wayland_socket}.{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        match UnixListener::bind(&path) {
            Ok(listener) => {
                let _ = listener.set_nonblocking(true);
                let added = handle.insert_source(Generic::new(listener, Interest::READ, Mode::Level), |_, listener, state: &mut State| {
                    while let Ok((stream, _)) = listener.accept() {
                        serve(state, stream);
                    }
                    Ok(PostAction::Continue)
                });
                if let Err(e) = added {
                    tracing::warn!("softcore msg won't work: {e}");
                }
            }
            Err(e) => tracing::warn!("softcore msg won't work: couldn't open {}: {e}", path.display()),
        }
        Ipc { path, subscribers: Vec::new(), last: Default::default() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Ipc {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn serve(state: &mut State, stream: UnixStream) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).is_err() {
        return;
    }
    let mut stream = stream;
    let args: Vec<String> = match serde_json::from_str(line.trim()) {
        Ok(a) => a,
        Err(_) => {
            let _ = writeln!(stream, "{}", json!({"error": "a request is a JSON array of words"}));
            return;
        }
    };
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    // Two requests keep their connection: the answer comes later (when PipeWire has made the
    // stream, when the person has chosen), and the connection closing means "never mind".
    match words.as_slice() {
        ["cast", "start", kind @ ("output" | "window"), target, rest @ ..] => {
            let source = match *kind {
                "output" => Some(crate::capture::Source::Output(target.to_string())),
                _ => target.parse().ok().map(crate::capture::Source::Window),
            };
            let started = match source {
                Some(s) => crate::cast::start(state, s, rest.contains(&"--cursor"), stream),
                None => Err((format!("{target:?} isn't a window's number"), stream)),
            };
            if let Err((e, mut stream)) = started {
                let _ = writeln!(stream, "{}", json!({"error": e}));
            }
            return;
        }
        ["pick-cast", rest @ ..] => {
            let value = |flag: &str| rest.iter().position(|w| *w == flag).and_then(|i| rest.get(i + 1)).copied();
            let types = value("--types").unwrap_or("output,window");
            crate::picker::open(state, types.contains("output"), types.contains("window"), value("--app").unwrap_or("").to_string(), stream);
            return;
        }
        _ => {}
    }
    if args.first().map(String::as_str) == Some("event-stream") {
        // What there is now, then the changes as they come.
        let first = [json!({"WorkspacesChanged": {"workspaces": workspaces(state)}}), json!({"WindowsChanged": {"windows": windows(state)}})];
        if first.iter().all(|e| writeln!(stream, "{e}").is_ok()) && stream.set_nonblocking(true).is_ok() {
            state.ipc.subscribers.push(stream);
        }
        return;
    }
    let reply = match request(state, &args) {
        Ok(v) => json!({"ok": v}),
        Err(e) => json!({"error": e}),
    };
    let _ = writeln!(stream, "{reply}");
}

fn request(state: &mut State, args: &[String]) -> Result<Value, String> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["version"] => Ok(json!(env!("CARGO_PKG_VERSION"))),
        ["outputs"] => Ok(outputs(state)),
        ["workspaces"] => Ok(workspaces(state)),
        ["windows"] => Ok(windows(state)),
        ["focused-window"] => {
            let id = state.layout.active_window();
            Ok(windows(state).as_array().and_then(|a| a.iter().find(|w| w["id"].as_u64() == id.map(u64::from)).cloned()).unwrap_or(Value::Null))
        }
        ["focused-output"] => {
            let name = state.active_output().map(|o| o.name());
            Ok(name.and_then(|n| outputs(state).get(&n).cloned()).unwrap_or(Value::Null))
        }
        ["casts"] => Ok(crate::cast::list(state)),
        ["cast", "stop", id] => {
            let id: u32 = id.parse().map_err(|_| format!("{id:?} isn't a cast's number"))?;
            if crate::cast::stop(state, id) { Ok(Value::Null) } else { Err(format!("no cast {id}")) }
        }
        ["screenshot", rest @ ..] => {
            state.do_action(Action::Screenshot(rest.first().map(|s| s.to_string())));
            Ok(Value::Null)
        }
        // niri's way of naming a window, which shells use to close or raise one.
        ["action", name @ ("close-window" | "focus-window"), "--id", id] => {
            let id: u32 = id.parse().map_err(|_| format!("{id:?} isn't a window's number"))?;
            let win = state.wins.get(&id).ok_or(format!("no window {id}"))?;
            if *name == "close-window" {
                if let Some(t) = win.window.toplevel() {
                    t.send_close();
                }
            } else {
                state.layout.focus_window(id);
                state.layer_focus = None;
                state.queue_redraw();
            }
            Ok(Value::Null)
        }
        ["action", rest @ ..] if !rest.is_empty() => {
            state.do_action(Action::parse(&rest.join(" "))?);
            Ok(Value::Null)
        }
        _ => Err(format!("softcore doesn't know {:?}", args.join(" "))),
    }
}

pub fn workspaces(state: &State) -> Value {
    let mut out = Vec::new();
    for (mi, m) in state.layout.mons.iter().enumerate() {
        for (wi, ws) in m.wss.iter().enumerate() {
            out.push(json!({
                "id": ws.id,
                "idx": wi + 1,
                "name": Value::Null,
                "output": m.name,
                "is_urgent": false,
                "is_active": wi == m.active,
                "is_focused": wi == m.active && mi == state.layout.active_mon,
                "active_window_id": ws.active_window(),
            }));
        }
    }
    Value::Array(out)
}

pub fn windows(state: &State) -> Value {
    let focused = state.layout.active_window();
    let mut ids: Vec<u32> = state.wins.keys().copied().collect();
    ids.sort_unstable();
    let mut out = Vec::new();
    for id in ids {
        let win = &state.wins[&id];
        let (title, app_id) = State::title_and_app_id(&win.window);
        let pid = win.window.toplevel().and_then(|t| state.dh.get_client(t.wl_surface().id()).ok()).and_then(|c| c.get_credentials(&state.dh).ok()).map(|c| c.pid);
        let ws = state.layout.find(id).map(|(mi, wi)| state.layout.mons[mi].wss[wi].id);
        let size = win.window.geometry().size;
        out.push(json!({
            "id": id,
            "title": title,
            "app_id": app_id,
            "pid": pid,
            "workspace_id": ws,
            "is_focused": focused == Some(id),
            "is_floating": state.layout.is_floating(id),
            "is_urgent": false,
            "layout": {
                "pos_in_scrolling_layout": state.layout.position(id).map(|(c, r)| json!([c, r])),
                "window_size": [size.w, size.h],
            },
            "focus_timestamp": {"secs": win.focused_at.as_secs(), "nanos": win.focused_at.subsec_nanos()},
        }));
    }
    Value::Array(out)
}

pub fn outputs(state: &State) -> Value {
    let mut out = serde_json::Map::new();
    for o in &state.outputs {
        let props = o.physical_properties();
        use smithay::utils::Transform as T;
        let transform = match o.current_transform() {
            T::Normal => "Normal",
            T::_90 => "90",
            T::_180 => "180",
            T::_270 => "270",
            T::Flipped => "Flipped",
            T::Flipped90 => "Flipped90",
            T::Flipped180 => "Flipped180",
            T::Flipped270 => "Flipped270",
        };
        // A window on another desktop is upside down to the renderer, not to anyone else.
        let transform = if o.name() == "winit" { "Normal" } else { transform };
        let modes = o.modes();
        let geo = State::output_geometry(o);
        out.insert(
            o.name(),
            json!({
                "name": o.name(),
                "make": props.make,
                "model": props.model,
                "serial": props.serial_number,
                "physical_size": [props.size.w, props.size.h],
                "modes": modes.iter().map(|m| json!({"width": m.size.w, "height": m.size.h, "refresh_rate": m.refresh, "is_preferred": Some(*m) == o.preferred_mode()})).collect::<Vec<_>>(),
                "current_mode": o.current_mode().and_then(|c| modes.iter().position(|m| *m == c)),
                "vrr_supported": false,
                "vrr_enabled": false,
                "logical": {"x": geo.loc.x, "y": geo.loc.y, "width": geo.size.w, "height": geo.size.h, "scale": o.current_scale().fractional_scale(), "transform": transform},
            }),
        );
    }
    // Plugged in and turned off: no mode in use, no place.
    for o in &state.off_outputs {
        out.insert(
            o.name.clone(),
            json!({
                "name": o.name, "make": o.make, "model": o.model, "serial": "Unknown", "physical_size": [0, 0],
                "modes": o.modes.iter().map(|(w, h, hz, own)| json!({"width": w, "height": h, "refresh_rate": hz, "is_preferred": own})).collect::<Vec<_>>(),
                "current_mode": null, "vrr_supported": false, "vrr_enabled": false, "logical": null,
            }),
        );
    }
    Value::Object(out)
}

/// Tell whoever is listening what changed since they were last told.
pub fn notify(state: &mut State) {
    if state.ipc.subscribers.is_empty() {
        return;
    }
    let ws = workspaces(state);
    let wins = windows(state);
    let focus = state.layout.active_window();
    let (ws_text, wins_text) = (ws.to_string(), wins.to_string());
    let mut events = Vec::new();
    if ws_text != state.ipc.last.0 {
        events.push(json!({"WorkspacesChanged": {"workspaces": ws}}));
    }
    if wins_text != state.ipc.last.1 {
        events.push(json!({"WindowsChanged": {"windows": wins}}));
    }
    if focus != state.ipc.last.2 {
        events.push(json!({"WindowFocusChanged": {"id": focus}}));
    }
    state.ipc.last = (ws_text, wins_text, focus);
    if events.is_empty() {
        return;
    }
    state.ipc.subscribers.retain_mut(|s| events.iter().all(|e| writeln!(s, "{e}").is_ok()));
}

/// `softcore msg [-j] WORDS…`: ask the running Softcore, print what it says.
pub fn client(args: &[String]) -> i32 {
    let json_out = args.iter().any(|a| a == "-j" || a == "--json");
    let words: Vec<&String> = args.iter().filter(|a| *a != "-j" && *a != "--json").collect();
    if words.is_empty() {
        eprintln!("softcore msg: say what to ask: outputs, workspaces, windows, focused-window, focused-output, action NAME, screenshot [FILE], casts, cast start output NAME|window ID [--cursor], cast stop ID, pick-cast [--types output,window] [--app NAME], event-stream, version");
        return 2;
    }
    let Some(path) = std::env::var_os("SOFTCORE_SOCKET") else {
        eprintln!("softcore msg: SOFTCORE_SOCKET isn't set. Is this running under Softcore?");
        return 1;
    };
    let mut stream = match UnixStream::connect(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("softcore msg: couldn't reach Softcore at {}: {e}", Path::new(&path).display());
            return 1;
        }
    };
    if writeln!(stream, "{}", json!(words)).is_err() {
        eprintln!("softcore msg: couldn't send the request");
        return 1;
    }
    let stream_events = words[0] == "event-stream";
    // A cast lasts as long as this connection: say its numbers, then stay until stopped.
    let hold = words.len() > 1 && words[0] == "cast" && words[1] == "start";
    for line in BufReader::new(stream).lines().map_while(Result::ok) {
        if stream_events {
            println!("{line}");
            continue;
        }
        let v: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
        if let Some(e) = v.get("error").and_then(Value::as_str) {
            eprintln!("softcore msg: {e}");
            return 1;
        }
        match &v["ok"] {
            Value::Null => {}
            ok if json_out => println!("{ok}"),
            ok => println!("{}", serde_json::to_string_pretty(ok).unwrap_or_default()),
        }
        if !hold {
            return 0;
        }
    }
    0
}
