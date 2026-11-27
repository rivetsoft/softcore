//! Softcore: the scrolling-tiling Wayland compositor for the Rivet desktop.

mod appearance;
mod backend;
mod blur;
mod capture;
mod chord;
mod cast;
mod config;
mod cursor;
mod devices;
mod float;
mod handlers;
mod host;
mod input;
mod install;
mod ipc;
mod layout;
mod overview;
mod picker;
mod portal;
mod render;
mod settings_cli;
mod shot;
mod state;
mod udev;
mod watch;
mod workspaces;
mod xwayland;
mod ui;

use std::os::fd::{AsRawFd, FromRawFd};
use std::time::Duration;

use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::{EventLoop, Interest, Mode, PostAction};
use smithay::reexports::wayland_server::Display;

use crate::state::State;

const HELP: &str = "\
Softcore, the scrolling-tiling Wayland compositor for the Rivet desktop.

Usage:
  softcore [OPTIONS]        Start Softcore
  softcore msg [-j] WORDS   Ask a running Softcore (softcore msg for the list)
  softcore settings         Show or change settings (softcore settings help)
  softcore portal           The service apps reach through xdg-desktop-portal: screen
                            sharing, screenshots, colors and dark or light
  softcore validate         Check the settings file and say what couldn't be read
  softcore install          Put the session's files in place; --system installs Softcore
                            for everyone and lists it in the login manager

Options:
  --tty                On displays of its own (the default from a TTY)
  --winit              In a window of the desktop that is running (the default there)
  --headless           With no screen: a display in memory, for checks and screenshots
  --size WIDTHxHEIGHT  The headless display's size (1280x800); more than one, split by
                       commas, for as many displays side by side
  --spawn COMMAND      Run a command once started (may be given more than once)
  --portal             Also run the screen-sharing service (always on, on displays of its own)
  -h, --help           This
  -V, --version        The version

Settings: ~/.config/softcore/settings (key=value; see the README).
";

enum Where {
    Auto,
    Winit,
    Headless,
    Tty,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("msg") => std::process::exit(ipc::client(&args[1..])),
        Some("portal") => std::process::exit(portal::run()),
        Some("install") => std::process::exit(install::run(&args[1..], false)),
        Some("uninstall") => std::process::exit(install::run(&args[1..], true)),
        Some("settings") => std::process::exit(settings_cli::run(&args[1..])),
        Some("validate") => {
            let c = config::load();
            for e in &c.errors {
                eprintln!("{e}");
            }
            if c.errors.is_empty() {
                println!("{}: nothing wrong", config::config_file().display());
            }
            std::process::exit(if c.errors.is_empty() { 0 } else { 1 });
        }
        Some("-h" | "--help" | "help") => {
            print!("{HELP}");
            return;
        }
        Some("-V" | "--version") => {
            println!("softcore {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        _ => {}
    }

    let mut place = Where::Auto;
    let mut sizes = vec![(1280, 800)];
    let mut spawn = Vec::new();
    let mut portal = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--winit" => place = Where::Winit,
            "--headless" => place = Where::Headless,
            "--tty" => place = Where::Tty,
            "--portal" => portal = true,
            "--size" => match it.next().and_then(|s| s.split(',').map(|s| s.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))).collect::<Option<Vec<_>>>()) {
                Some(s) => sizes = s,
                None => fail("--size needs a size like 1280x800, or sizes like 1280x800,1920x1080"),
            },
            "--spawn" => match it.next() {
                Some(c) => spawn.push(c.clone()),
                None => fail("--spawn needs a command"),
            },
            other => fail(&format!("softcore doesn't know {other:?} (softcore --help)")),
        }
    }

    if let Ok(filter) = tracing_subscriber::EnvFilter::try_from_default_env() {
        tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
    } else {
        tracing_subscriber::fmt().with_env_filter("warn,softcore=info").with_writer(std::io::stderr).init();
    }
    // Programs Softcore starts aren't waited for; this lets them go when they finish.
    unsafe {
        libc::signal(libc::SIGCHLD, libc::SIG_IGN);
    }

    let has_desktop = std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some();
    let place = match place {
        Where::Auto if has_desktop => Where::Winit,
        Where::Auto => Where::Tty,
        p => p,
    };

    let mut event_loop: EventLoop<'static, State> = EventLoop::try_new().unwrap_or_else(|e| fail(&format!("couldn't start: {e}")));
    let display: Display<State> = Display::new().unwrap_or_else(|e| fail(&format!("couldn't start: {e}")));
    let config = config::load();
    let mut state = State::new(display, event_loop.handle(), event_loop.get_signal(), !matches!(place, Where::Tty), config);

    let started = match place {
        Where::Headless => backend::init_headless(&event_loop, &mut state, &sizes),
        Where::Tty => udev::init(&event_loop, &mut state),
        _ => backend::init_winit(&event_loop, &mut state),
    };
    if let Err(e) = started {
        fail(&e);
    }

    event_loop
        .handle()
        .insert_source(Timer::from_duration(Duration::from_secs(2)), |_, _, state| {
            state.check_files();
            TimeoutAction::ToDuration(Duration::from_secs(2))
        })
        .expect("a timer");
    watch::start(&event_loop.handle(), &mut state);

    xwayland::start(&mut state);
    tracing::info!("Softcore is up: WAYLAND_DISPLAY={} SOFTCORE_SOCKET={}", state.socket, state.ipc.path().display());
    // For whoever started it (a script, a check) to find it.
    println!("WAYLAND_DISPLAY={}\nSOFTCORE_SOCKET={}", state.socket, state.ipc.path().display());

    if let Some(x11) = &state.x11 {
        println!("DISPLAY=:{}", x11.display);
    }
    if !state.config.errors.is_empty() {
        state.ui.show(ui::Panel::Errors);
    }
    // The screen-sharing service first, so it is there when the desktop's portal looks for it.
    if (portal || !state.nested)
        && let Ok(exe) = std::env::current_exe()
    {
        state.spawn(&format!("exec '{}' portal", exe.display().to_string().replace('\'', "'\\''")));
    }
    // On displays of its own Softcore is the session. D-Bus and systemd are told where it is,
    // so what they start (the portal, apps) finds it; then the session's target is started,
    // which is what the desktop's portal and other session services wait for. A mark says
    // this Softcore started it, so a second one beside it doesn't end the first's session.
    let session_mark = (!state.nested).then(|| std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir).join(format!("softcore-session.{}", std::process::id())));
    if let Some(mark) = &session_mark {
        let display = if state.x11.is_some() { " DISPLAY" } else { "" };
        state.spawn(&format!(
            "dbus-update-activation-environment --systemd WAYLAND_DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_DESKTOP XDG_SESSION_TYPE SOFTCORE_SOCKET{display}; systemctl --user is-active -q softcore-session.target || {{ systemctl --user start softcore-session.target && : > '{}'; }}",
            mark.display()
        ));
    }
    for c in state.config.spawn.clone().iter().chain(&spawn) {
        state.spawn(c);
    }

    // Asked to stop (a kill, Ctrl+C in the terminal it was started from): leave as tidily as
    // through the leave key, so the sockets it made don't stay behind. The signal handler
    // only writes to an eventfd, which wakes the loop.
    static STOP: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);
    extern "C" fn stop(_: libc::c_int) {
        let one = 1u64;
        unsafe {
            libc::write(STOP.load(std::sync::atomic::Ordering::Relaxed), (&one as *const u64).cast(), 8);
        }
    }
    let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    if fd < 0 {
        fail(&format!("couldn't start: {}", std::io::Error::last_os_error()));
    }
    // Safety: just made, and owned by nothing else. Kept open as long as the loop is, so a
    // second signal never writes anywhere else.
    let fd = unsafe { std::fs::File::from_raw_fd(fd) };
    STOP.store(fd.as_raw_fd(), std::sync::atomic::Ordering::Relaxed);
    event_loop
        .handle()
        .insert_source(Generic::new(fd, Interest::READ, Mode::Level), |_, fd, state| {
            let _ = std::io::Read::read(&mut &**fd, &mut [0u8; 8]);
            state.signal.stop();
            Ok(PostAction::Continue)
        })
        .expect("a stop signal");
    unsafe {
        libc::signal(libc::SIGTERM, stop as extern "C" fn(libc::c_int) as libc::sighandler_t);
        libc::signal(libc::SIGINT, stop as extern "C" fn(libc::c_int) as libc::sighandler_t);
    }

    let ran = event_loop.run(None, &mut state, |state| state.refresh());
    // The session ends with the Softcore that began it, and systemd forgets where it was.
    if let Some(mark) = session_mark.filter(|m| m.exists()) {
        let _ = std::fs::remove_file(&mark);
        let _ = std::process::Command::new("sh")
            .arg("-c")
            .arg("systemctl --user stop softcore-session.target; systemctl --user unset-environment WAYLAND_DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_DESKTOP XDG_SESSION_TYPE SOFTCORE_SOCKET DISPLAY")
            .spawn();
    }
    if let Err(e) = ran {
        fail(&format!("Softcore stopped: {e}"));
    }
}

fn fail(message: &str) -> ! {
    eprintln!("softcore: {message}");
    std::process::exit(1)
}
