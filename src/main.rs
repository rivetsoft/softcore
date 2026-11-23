//! Softcore: the scrolling-tiling Wayland compositor for the Rivet desktop.

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

use std::time::Duration;

use smithay::reexports::calloop::EventLoop;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::wayland_server::Display;

use crate::state::State;

const HELP: &str = "\
Softcore, the scrolling-tiling Wayland compositor for the Rivet desktop.

Usage:
  softcore [OPTIONS]        Start Softcore
  softcore msg [-j] WORDS   Ask a running Softcore (softcore msg for the list)
  softcore settings         Show or change settings (softcore settings help)
  softcore portal           The screen-sharing service apps reach through xdg-desktop-portal
  softcore validate         Check the settings file and say what couldn't be read

Options:
  --tty                On displays of its own (the default from a TTY)
  --winit              In a window of the desktop that is running (the default there)
  --headless           With no screen: a display in memory, for checks and screenshots
  --size WIDTHxHEIGHT  The headless display's size (1280x800)
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
    let mut size = (1280, 800);
    let mut spawn = Vec::new();
    let mut portal = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--winit" => place = Where::Winit,
            "--headless" => place = Where::Headless,
            "--tty" => place = Where::Tty,
            "--portal" => portal = true,
            "--size" => match it.next().and_then(|s| s.split_once('x')).and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))) {
                Some(s) => size = s,
                None => fail("--size needs a size like 1280x800"),
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
        Where::Headless => backend::init_headless(&event_loop, &mut state, size),
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
    // On displays of its own Softcore is the session: tell D-Bus and systemd where it is, so
    // what they start (the portal, apps) finds it, and run the screen-sharing service.
    if !state.nested {
        let display = if state.x11.is_some() { " DISPLAY" } else { "" };
        state.spawn(&format!("dbus-update-activation-environment --systemd WAYLAND_DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_DESKTOP XDG_SESSION_TYPE SOFTCORE_SOCKET{display}"));
    }
    if (portal || !state.nested)
        && let Ok(exe) = std::env::current_exe()
    {
        state.spawn(&format!("exec '{}' portal", exe.display().to_string().replace('\'', "'\\''")));
    }
    for c in state.config.spawn.clone().iter().chain(&spawn) {
        state.spawn(c);
    }

    // Asked to stop (a kill, Ctrl+C in the terminal it was started from): leave as tidily as
    // through the leave key, so the sockets it made don't stay behind.
    static STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    extern "C" fn stop(_: libc::c_int) {
        STOP.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    unsafe {
        libc::signal(libc::SIGTERM, stop as extern "C" fn(libc::c_int) as libc::sighandler_t);
        libc::signal(libc::SIGINT, stop as extern "C" fn(libc::c_int) as libc::sighandler_t);
    }
    event_loop
        .handle()
        .insert_source(Timer::from_duration(Duration::from_millis(100)), |_, _, state| {
            if STOP.load(std::sync::atomic::Ordering::Relaxed) {
                state.signal.stop();
            }
            TimeoutAction::ToDuration(Duration::from_millis(100))
        })
        .expect("a timer");

    if let Err(e) = event_loop.run(None, &mut state, |state| state.refresh()) {
        fail(&format!("Softcore stopped: {e}"));
    }
}

fn fail(message: &str) -> ! {
    eprintln!("softcore: {message}");
    std::process::exit(1)
}
