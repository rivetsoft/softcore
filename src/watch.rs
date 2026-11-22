//! Told at once when a file Softcore follows changes: its settings, the desktop's colours,
//! the desktop's light or dark choice. A new theme chosen in Rivet Shell recolours the
//! borders and panels in the same moment, not at the next look every two seconds (which
//! stays, for files in places that can't be watched).

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::PathBuf;

use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, LoopHandle, Mode, PostAction};

use crate::state::State;

pub struct Watch {
    fd: i32,
}

/// The folders holding the files that matter now. Folders, since a file is often replaced by
/// a new one of the same name rather than written into.
fn folders(state: &State) -> Vec<PathBuf> {
    let config = rivet_xdg::places::config_home();
    let mut files: Vec<PathBuf> = state.config.files.iter().map(|(p, _)| p.clone()).collect();
    files.extend(rivet_look::palette::load().1);
    files.extend(["gtk-3.0/settings.ini", "gtk-4.0/settings.ini", "rivet/colors.css", "rivet/palette.css"].map(|f| config.join(f)));
    let mut out: Vec<PathBuf> = Vec::new();
    for f in files {
        // Where it is named, and where it really is when that is a link elsewhere.
        for p in [Some(f.clone()), std::fs::canonicalize(&f).ok()].into_iter().flatten() {
            // Its folder, or while that isn't there yet, the nearest one that is: the folder
            // appearing is then noticed too.
            if let Some(dir) = p.ancestors().skip(1).find(|d| d.is_dir())
                && !out.iter().any(|o| o == dir)
            {
                out.push(dir.to_path_buf());
            }
        }
    }
    out
}

impl Watch {
    /// Watch the folders that matter now (again after every change: a file may name others).
    pub fn update(&self, state: &State) {
        let mask = libc::IN_CLOSE_WRITE | libc::IN_MOVED_TO | libc::IN_CREATE | libc::IN_DELETE | libc::IN_MODIFY;
        for dir in folders(state) {
            if let Ok(c) = std::ffi::CString::new(dir.as_os_str().as_encoded_bytes()) {
                // Watching a folder twice is the same watch.
                unsafe { libc::inotify_add_watch(self.fd, c.as_ptr(), mask) };
            }
        }
    }
}

pub fn start(handle: &LoopHandle<'static, State>, state: &mut State) {
    let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
    if fd < 0 {
        tracing::warn!("files can't be watched; changes are noticed within two seconds instead");
        return;
    }
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    let watch = Watch { fd };
    watch.update(state);
    state.watch = Some(watch);
    let added = handle.insert_source(Generic::new(owned, Interest::READ, Mode::Level), |_, fd, state| {
        // What changed doesn't matter, only that something did.
        let mut buf = [0u8; 4096];
        while unsafe { libc::read(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) } > 0 {}
        state.check_files();
        if let Some(w) = state.watch.take() {
            w.update(state);
            state.watch = Some(w);
        }
        // Once more: a file written into a folder that was only just made, before the
        // folder was watched, would otherwise wait for the two seconds.
        state.check_files();
        Ok(PostAction::Continue)
    });
    if added.is_err() {
        state.watch = None;
    }
}
