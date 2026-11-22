//! Softcore's settings. The file's format, what each key is, and changing it in place are
//! the `softcore-settings` crate's, so a settings window uses the same code; this adds what
//! only the compositor can check.

pub use softcore_settings::*;

/// The settings on disk, with bindings that name no key reported.
pub fn load() -> Config {
    let mut c = Config::load();
    let unknown: Vec<String> = c.binds.iter().flat_map(|b| b.keys().into_iter().filter(|(_, k)| crate::input::key_sym(k).is_none()).map(move |(_, k)| format!("bind.{}: there is no key called {k:?}", b.shown))).collect();
    c.errors.extend(unknown);
    c
}
