//! `softcore install`: the files a session needs outside Softcore itself, which a package
//! would otherwise put in place. Without them apps can't reach the desktop's portal (the
//! file picker, screen sharing), and login managers don't list Softcore.
//!
//! For one person it writes the session's files under their home. With `--system`, as root,
//! it installs Softcore for everyone: the program itself, those files, and the entry that
//! puts Softcore in a login manager's list of sessions.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const TARGET: &str = include_str!("../data/softcore-session.target");
const PORTAL: &str = include_str!("../data/softcore.portal");
const PORTALS_CONF: &str = include_str!("../data/softcore-portals.conf");
const DESKTOP: &str = include_str!("../data/softcore.desktop");

/// Where the program goes for everyone.
const PROGRAM: &str = "/usr/local/bin/softcore";

/// A place on this machine, or under `DESTDIR` for someone making a package.
fn rooted(path: &str) -> PathBuf {
    match std::env::var_os("DESTDIR").filter(|d| !d.is_empty()) {
        Some(d) => PathBuf::from(d).join(path.trim_start_matches('/')),
        None => PathBuf::from(path),
    }
}

/// Where each file goes, and what is in it.
fn files(system: bool) -> Vec<(PathBuf, String)> {
    if system {
        vec![
            (rooted("/usr/lib/systemd/user/softcore-session.target"), TARGET.into()),
            (rooted("/usr/share/xdg-desktop-portal/portals/softcore.portal"), PORTAL.into()),
            (rooted("/usr/share/xdg-desktop-portal/softcore-portals.conf"), PORTALS_CONF.into()),
            // A login manager starts it by its whole path: its own PATH may not reach it.
            (rooted("/usr/share/wayland-sessions/softcore.desktop"), DESKTOP.replace("Exec=softcore", &format!("Exec={PROGRAM}"))),
        ]
    } else {
        let (config, data) = (rivet_xdg::places::config_home(), rivet_xdg::places::data_home());
        vec![
            (config.join("systemd/user/softcore-session.target"), TARGET.into()),
            (data.join("xdg-desktop-portal/portals/softcore.portal"), PORTAL.into()),
            (config.join("xdg-desktop-portal/softcore-portals.conf"), PORTALS_CONF.into()),
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

/// Put this program where everyone can run it. Written beside its place and moved in, so a
/// Softcore that is running from there goes on running the one it started as.
fn program(to: &Path) -> Result<&'static str, String> {
    let from = std::env::current_exe().map_err(|e| format!("couldn't find this program: {e}"))?;
    if from.canonicalize().ok() == to.canonicalize().ok() {
        return Ok("this is it");
    }
    let bytes = std::fs::read(&from).map_err(|e| format!("couldn't read {}: {e}", from.display()))?;
    if std::fs::read(to).is_ok_and(|have| have == bytes) {
        return Ok("as it should be");
    }
    let was_there = to.exists();
    if let Some(d) = to.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("couldn't make {}: {e}", d.display()))?;
    }
    let beside = to.with_extension("new");
    std::fs::write(&beside, &bytes).and_then(|_| std::fs::set_permissions(&beside, std::fs::Permissions::from_mode(0o755))).and_then(|_| std::fs::rename(&beside, to)).map_err(|e| {
        let _ = std::fs::remove_file(&beside);
        format!("couldn't write {}: {e}", to.display())
    })?;
    Ok(if was_there { "brought up to date" } else { "written" })
}

const USAGE: &str = "\
softcore install              For you: the session target and the portal files
softcore install --system     For everyone, as root: the program, those files, and the entry
                              that puts Softcore in the login manager's list
softcore uninstall [--system] Take them away again
";

pub fn run(args: &[String], remove: bool) -> i32 {
    let system = match args {
        [] => false,
        [a] if a == "--system" => true,
        _ => {
            eprint!("{USAGE}");
            return 2;
        }
    };
    let mut failed = false;
    let mut say = |path: &Path, done: Result<&str, String>| match done {
        Ok(how) => println!("{}: {how}", path.display()),
        Err(e) => {
            eprintln!("softcore: {e}");
            failed = true;
        }
    };
    let mut paths: Vec<(PathBuf, String)> = files(system);
    if remove {
        if system {
            paths.push((rooted(PROGRAM), String::new()));
        }
        for (path, _) in &paths {
            let done = match std::fs::remove_file(path) {
                Ok(()) => Ok("taken away"),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok("wasn't there"),
                Err(e) => Err(format!("couldn't take {} away: {e}", path.display())),
            };
            say(path, done);
        }
    } else {
        if system {
            let to = rooted(PROGRAM);
            say(&to, program(&to));
        }
        for (path, text) in &paths {
            say(path, write(path, text));
        }
    }
    if !system {
        // systemd reads unit files when told to.
        let _ = std::process::Command::new("systemctl").args(["--user", "daemon-reload"]).status();
    }
    if failed {
        if system && unsafe { libc::geteuid() } != 0 && std::env::var_os("DESTDIR").is_none() {
            eprintln!("\nInstalling for everyone needs root: sudo {} install --system", std::env::current_exe().map(|e| e.display().to_string()).unwrap_or_else(|_| "softcore".into()));
        }
        return 1;
    }
    if !remove {
        println!("{}", if system { "\nSoftcore is in the login manager's list of sessions from the next time you log out." } else { "\nThis takes effect the next time Softcore starts on displays of its own.\nTo install Softcore for everyone and list it in the login manager: sudo softcore install --system" });
    }
    0
}
