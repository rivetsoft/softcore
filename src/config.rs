//! Softcore's settings. The file's format, what each key is, and changing it in place are
//! the `softcore-settings` crate's, so a settings window uses the same code; this adds what
//! only the compositor can check.

pub use softcore_settings::*;

/// The settings on disk, with bindings that name no key reported.
pub fn load() -> Config {
    let mut c = Config::load();
    let unknown: Vec<String> = c.binds.iter().filter(|b| crate::input::key_sym(&b.key).is_none()).map(|b| format!("bind.{}: there is no key called {:?}", b.shown, b.key)).collect();
    c.errors.extend(unknown);
    c
}
