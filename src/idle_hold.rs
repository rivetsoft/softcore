//! Keeping the screen awake for an app that isn't a window in front (the inhibit portal: a
//! video player in the background, a download). `inhibit-idle` holds idle off while its
//! connection stays open, as an idle-inhibiting surface does while it is shown.

use std::io::Write;
use std::os::unix::net::UnixStream;

use crate::state::State;

#[derive(Default)]
pub struct IdleHolds {
    held: Vec<(UnixStream, crate::state::Hangup)>,
}

impl IdleHolds {
    pub fn any(&self) -> bool {
        !self.held.is_empty()
    }
}

pub fn hold(state: &mut State, mut stream: UnixStream) {
    if writeln!(stream, "{}", serde_json::json!({"ok": null})).is_err() {
        return;
    }
    let _ = stream.set_nonblocking(true);
    let hangup = crate::state::Hangup::new(&state.handle, &stream);
    state.idle_holds.held.push((stream, hangup));
    update(state);
}

/// Whether anything is holding idle off: a surface that asked, or an app through the portal.
pub fn update(state: &mut State) {
    let any = !state.idle_inhibitors.is_empty() || state.idle_holds.any();
    state.idle_notifier_state.set_is_inhibited(any);
}

/// Once round the loop: a hold whose connection closed is let go.
pub fn refresh(state: &mut State) {
    let before = state.idle_holds.held.len();
    state.idle_holds.held.retain_mut(|(s, _)| match std::io::Read::read(s, &mut [0u8; 16]) {
        Ok(0) => false,
        Err(e) => e.kind() == std::io::ErrorKind::WouldBlock,
        Ok(_) => true,
    });
    if state.idle_holds.held.len() != before {
        update(state);
    }
}
