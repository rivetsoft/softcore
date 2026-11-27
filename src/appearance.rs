//! The desktop's appearance for apps that aren't Rivet's: `org.freedesktop.impl.portal.Settings`,
//! answered by `softcore portal`. GTK 4, libadwaita, Qt, Firefox and Electron apps ask
//! xdg-desktop-portal whether to be dark, which accent to use and whether to raise the
//! contrast, and change when told the answer changed.
//!
//! The answers come from where Rivet's own apps and Softcore's panels take theirs: the light or
//! dark choice is Softcore's `theme` setting, and with `system` the desktop's GTK settings
//! (dark when they don't say, as in Rivet); the accent is the desktop's generated colors when
//! they are followed (~/.config/rivet/colors.css, then the GTK color files), else `accent`.

use std::collections::HashMap;

use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};

pub const NAMESPACE: &str = "org.freedesktop.appearance";

/// What apps are told, in the portal's terms.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Appearance {
    /// 1: prefer dark, 2: prefer light (0, no preference, is never said: Rivet always has one).
    pub color_scheme: u32,
    /// Red, green and blue from 0 to 1.
    pub accent: (f64, f64, f64),
    /// 1: higher contrast.
    pub contrast: u32,
}

impl Appearance {
    pub fn now() -> Appearance {
        let c = softcore_settings::Config::load();
        let dark = match c.theme {
            softcore_settings::Theme::Dark => true,
            softcore_settings::Theme::Light => false,
            softcore_settings::Theme::System => rivet_look::palette::prefers_dark(),
        };
        let (palette, _) = rivet_look::palette::load();
        let accent = rivet_look::palette::shown(palette.as_ref(), c.follow_desktop_colors).and_then(|p| p.accent).map(|a| [a.r(), a.g(), a.b()]).unwrap_or(c.accent);
        let f = |v: u8| v as f64 / 255.0;
        Appearance { color_scheme: if dark { 1 } else { 2 }, accent: (f(accent[0]), f(accent[1]), f(accent[2])), contrast: high_contrast() as u32 }
    }

    /// Each key and its value.
    pub fn values(&self) -> [(&'static str, OwnedValue); 3] {
        [("color-scheme", OwnedValue::from(self.color_scheme)), ("accent-color", owned(Value::from(self.accent))), ("contrast", OwnedValue::from(self.contrast))]
    }
}

fn owned(v: Value<'_>) -> OwnedValue {
    v.try_to_owned().expect("a value with no file descriptors can be kept")
}

/// Whether the desktop's GTK theme is a high-contrast one, the only place a Rivet desktop says so.
fn high_contrast() -> bool {
    let c = rivet_xdg::places::config_home();
    ["gtk-4.0/settings.ini", "gtk-3.0/settings.ini"].iter().filter_map(|f| std::fs::read_to_string(c.join(f)).ok()).flat_map(|t| t.lines().map(str::to_string).collect::<Vec<_>>()).any(|l| {
        l.trim().strip_prefix("gtk-theme-name").is_some_and(|v| v.trim_start_matches([' ', '=']).trim().to_lowercase().contains("highcontrast"))
    })
}

/// Whether a namespace asked for (`ReadAll`'s list, where a trailing `*` matches the rest) is ours.
fn wanted(patterns: &[String]) -> bool {
    patterns.is_empty() || patterns.iter().any(|p| p.is_empty() || p == NAMESPACE || p.strip_suffix('*').is_some_and(|start| NAMESPACE.starts_with(start)))
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.portal.Error")]
pub enum Error {
    #[zbus(error)]
    ZBus(zbus::Error),
    NotFound(String),
}

pub struct Settings;

#[zbus::interface(name = "org.freedesktop.impl.portal.Settings")]
impl Settings {
    async fn read_all(&self, namespaces: Vec<String>) -> HashMap<String, HashMap<String, OwnedValue>> {
        let mut out = HashMap::new();
        if wanted(&namespaces) {
            let now = blocking::unblock(Appearance::now).await;
            out.insert(NAMESPACE.to_string(), now.values().into_iter().map(|(k, v)| (k.to_string(), v)).collect());
        }
        out
    }

    async fn read(&self, namespace: String, key: String) -> Result<OwnedValue, Error> {
        if namespace == NAMESPACE {
            let now = blocking::unblock(Appearance::now).await;
            if let Some((_, v)) = now.values().into_iter().find(|(k, _)| *k == key) {
                return Ok(v);
            }
        }
        Err(Error::NotFound(format!("{namespace} {key} isn't a setting Softcore has")))
    }

    #[zbus(signal)]
    pub async fn setting_changed(emitter: &SignalEmitter<'_>, namespace: &str, key: &str, value: Value<'_>) -> zbus::Result<()>;

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }
}

/// Look again every two seconds, as Rivet's apps do, and tell apps what changed. Returns when
/// the session's bus has gone, so the service goes with the session.
pub fn watch(connection: zbus::Connection) {
    let Ok(emitter) = SignalEmitter::new(&connection, crate::portal::PATH) else { return };
    let Ok(bus) = zbus::blocking::fdo::DBusProxy::new(&zbus::blocking::Connection::from(connection.clone())) else { return };
    let mut last = Appearance::now();
    loop {
        std::thread::sleep(std::time::Duration::from_secs(2));
        if bus.get_id().is_err() {
            return;
        }
        let now = Appearance::now();
        if now == last {
            continue;
        }
        for ((key, value), (_, was)) in now.values().into_iter().zip(last.values()) {
            if value != was {
                let _ = zbus::block_on(Settings::setting_changed(&emitter, NAMESPACE, key, Value::from(value)));
            }
        }
        last = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespaces_are_matched_whole_or_by_what_they_start_with() {
        let w = |p: &[&str]| wanted(&p.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert!(w(&[]));
        assert!(w(&[""]));
        assert!(w(&["org.freedesktop.appearance"]));
        assert!(w(&["org.freedesktop.*"]));
        assert!(w(&["org.gnome.desktop.interface", "org.freedesktop.appearance"]));
        assert!(!w(&["org.gnome.*"]));
        assert!(!w(&["org.freedesktop.appearance.more"]));
    }
}
