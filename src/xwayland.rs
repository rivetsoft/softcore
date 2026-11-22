//! X11 programs, through xwayland-satellite.
//!
//! Softcore speaks no X11 itself. It takes a display number and listens on its sockets; when
//! the first X11 program knocks, it starts xwayland-satellite and hands the sockets over.
//! Satellite runs Xwayland and shows each X11 window as an ordinary Wayland one, so nothing
//! else in Softcore knows the difference. If satellite stops, Softcore listens again.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::{SocketAddr, UnixListener};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::time::Instant;

use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, Mode, PostAction, RegistrationToken};

use crate::state::State;

const PROGRAM: &str = "xwayland-satellite";
const DIR: &str = "/tmp/.X11-unix";

pub struct X11 {
    pub display: u32,
    lock: PathBuf,
    socket: PathBuf,
    /// The one in the file system, and the abstract one older programs look for.
    listeners: [UnixListener; 2],
    watching: [Option<RegistrationToken>; 2],
    started: Instant,
    /// Times in a row satellite stopped as soon as it started.
    failed: u32,
}

impl Drop for X11 {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket);
        let _ = std::fs::remove_file(&self.lock);
    }
}

fn installed() -> bool {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path).any(|d| d.join(PROGRAM).is_file())
}

/// Takes the lock file of display `n`, as X servers do: the file holds the owner's pid. One
/// left behind by a process that is gone is taken over.
fn take_lock(n: u32) -> Option<PathBuf> {
    use std::io::Write;
    let path = PathBuf::from(format!("/tmp/.X{n}-lock"));
    for _ in 0..2 {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                if f.write_all(format!("{:>10}\n", std::process::id()).as_bytes()).is_ok() {
                    return Some(path);
                }
                let _ = std::fs::remove_file(&path);
                return None;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                let owner = std::fs::read_to_string(&path).ok().and_then(|s| s.trim().parse::<i32>().ok());
                let gone = owner.is_some_and(|pid| unsafe { libc::kill(pid, 0) } != 0 && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH));
                if !gone || std::fs::remove_file(&path).is_err() {
                    return None;
                }
            }
            Err(_) => return None,
        }
    }
    None
}

fn open() -> Option<X11> {
    let _ = std::fs::create_dir(DIR);
    for display in 0..64 {
        let Some(lock) = take_lock(display) else { continue };
        let socket = PathBuf::from(format!("{DIR}/X{display}"));
        let _ = std::fs::remove_file(&socket);
        let both = UnixListener::bind(&socket).and_then(|file| {
            let name = SocketAddr::from_abstract_name(socket.as_os_str().as_encoded_bytes())?;
            Ok([file, UnixListener::bind_addr(&name)?])
        });
        match both {
            Ok(listeners) => return Some(X11 { display, lock, socket, listeners, watching: [None, None], started: Instant::now(), failed: 0 }),
            Err(_) => {
                let _ = std::fs::remove_file(&socket);
                let _ = std::fs::remove_file(&lock);
            }
        }
    }
    None
}

/// Takes a display number and waits for the first X11 program.
pub fn start(state: &mut State) {
    if !state.config.xwayland {
        return;
    }
    if !installed() {
        tracing::info!("no {PROGRAM} to be found: X11 programs won't open");
        return;
    }
    let Some(x11) = open() else {
        tracing::warn!("couldn't take an X11 display number: X11 programs won't open");
        return;
    };
    state.x11 = Some(x11);
    watch(state);
}

fn watch(state: &mut State) {
    let handle = state.handle.clone();
    let Some(x11) = state.x11.as_mut() else { return };
    for i in 0..2 {
        let Ok(copy) = x11.listeners[i].try_clone() else { continue };
        x11.watching[i] = handle
            .insert_source(Generic::new(copy, Interest::READ, Mode::Level), move |_, _, state| {
                // This one goes by what it returns, the other by hand.
                if let Some(x11) = state.x11.as_mut() {
                    x11.watching[i] = None;
                    if let Some(other) = x11.watching[1 - i].take() {
                        state.handle.remove(other);
                    }
                }
                run(state);
                Ok(PostAction::Remove)
            })
            .ok();
    }
}

/// Starts satellite on the sockets, and listens again once it has gone.
fn run(state: &mut State) {
    let Some(x11) = state.x11.as_mut() else { return };
    let fds = [x11.listeners[0].as_raw_fd(), x11.listeners[1].as_raw_fd()];
    let mut c = std::process::Command::new(PROGRAM);
    c.arg(format!(":{}", x11.display));
    for fd in fds {
        c.arg("-listenfd").arg(fd.to_string());
    }
    c.env("WAYLAND_DISPLAY", &state.socket).env_remove("DISPLAY").stdin(std::process::Stdio::null());
    unsafe {
        c.pre_exec(move || {
            // Softcore doesn't wait for what it starts; satellite waits for Xwayland.
            libc::signal(libc::SIGCHLD, libc::SIG_DFL);
            for fd in fds {
                if libc::fcntl(fd, libc::F_SETFD, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    let child = match c.spawn() {
        Ok(child) => child,
        Err(e) => {
            tracing::warn!("couldn't start {PROGRAM}: {e}");
            return;
        }
    };
    x11.started = Instant::now();
    tracing::info!("an X11 program asked for display :{}: started {PROGRAM}", x11.display);

    // Softcore's children are reaped for it, so there is nothing to wait on: a pidfd says
    // when this one has gone.
    let pidfd = unsafe { libc::syscall(libc::SYS_pidfd_open, child.id() as libc::pid_t, 0) };
    if pidfd < 0 {
        tracing::warn!("can't tell when {PROGRAM} stops: it won't be started again");
        return;
    }
    let pidfd = unsafe { OwnedFd::from_raw_fd(pidfd as i32) };
    let added = state.handle.insert_source(Generic::new(pidfd, Interest::READ, Mode::Level), |_, _, state| {
        if let Some(x11) = state.x11.as_mut() {
            x11.failed = if x11.started.elapsed().as_secs() < 2 { x11.failed + 1 } else { 0 };
            if x11.failed >= 3 {
                tracing::warn!("{PROGRAM} keeps stopping as soon as it starts: X11 programs won't open");
            } else {
                watch(state);
            }
        }
        Ok(PostAction::Remove)
    });
    if added.is_err() {
        tracing::warn!("can't tell when {PROGRAM} stops: it won't be started again");
    }
}
