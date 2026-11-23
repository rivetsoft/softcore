//! `softcore install`: the files a session needs outside Softcore itself, which a package
//! would otherwise put in place. Without them apps can't reach the desktop's portal (the
//! file picker, screen sharing), and login managers don't list Softcore.

use std::path::{Path, PathBuf};

const TARGET: &str = include_str!("../data/softcore-session.target");
const PORTAL: &str = include_str!("../data/softcore.portal");
const PORTALS_CONF: &str = include_str!("../data/softcore-portals.conf");
const DESKTOP: &str = include_str!("../data/softcore.desktop");

/// Where each file goes: for this user, or with `--system` for everyone (as root).
fn files(system: bool) -> Vec<(PathBuf, &'static str)> {
    if system {
        vec![
            (PathBuf::from("/usr/lib/systemd/user/softcore-session.target"), TARGET),
            (PathBuf::from("/usr/share/xdg-desktop-portal/portals/softcore.portal"), PORTAL),
            (PathBuf::from("/usr/share/xdg-desktop-portal/softcore-portals.conf"), PORTALS_CONF),
            (PathBuf::from("/usr/share/wayland-sessions/softcore.desktop"), DESKTOP),
        ]
    } else {
        let (config, data) = (rivet_xdg::places::config_home(), rivet_xdg::places::data_home());
        vec![
            (config.join("systemd/user/softcore-session.target"), TARGET),
            (data.join("xdg-desktop-portal/portals/softcore.portal"), PORTAL),
            (config.join("xdg-desktop-portal/softcore-portals.conf"), PORTALS_CONF),
        ]
    }
}

fn write(path: &Path, text: &str) -> Result<&'static str, String> {
    match std::fs::read_to_string(path) {
        Ok(have) if have == text => return Ok("as it should be"),
        _ => {}
    }
    let was_there = path.exists();
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("couldn't make {}: {e}", d.display()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("couldn't write {}: {e}", path.display()))?;
    Ok(if was_there { "brought up to date" } else { "written" })
}

pub fn run(args: &[String]) -> i32 {
    let system = match args {
        [] => false,
        [a] if a == "--system" => true,
        _ => {
            eprintln!("softcore install            For you: the session target and the portal files\nsoftcore install --system   For everyone, as root: those, and the entry login managers list");
            return 2;
        }
    };
    let mut failed = false;
    for (path, text) in files(system) {
        match write(&path, text) {
            Ok(how) => println!("{}: {how}", path.display()),
            Err(e) => {
                eprintln!("softcore install: {e}");
                failed = true;
            }
        }
    }
    if !system {
        // systemd reads unit files when told to.
        let _ = std::process::Command::new("systemctl").args(["--user", "daemon-reload"]).status();
        println!("\nThis takes effect the next time Softcore starts on displays of its own.\nFor login managers to list Softcore: sudo softcore install --system");
    }
    if failed { 1 } else { 0 }
}
