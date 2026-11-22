//! The settings file as lines, changed one at a time.

use std::path::{Path, PathBuf};

use crate::parse_combo;

/// A line's key and value, when it is a setting (not a comment, not blank).
pub(crate) fn entry(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    line.split_once('=').map(|(k, v)| (k.trim(), v.trim()))
}

/// A key as it is compared: a binding's keys in their one spelling, so `bind.shift+mod+q`
/// and `bind.Mod+Shift+Q` are the same line.
fn canon(key: &str) -> String {
    match key.strip_prefix("bind.").and_then(|c| parse_combo(c).ok()) {
        Some((mods, k)) => format!("bind.{}", mods.combo(&k)),
        None => key.to_string(),
    }
}

/// A settings file open for changing. Comments, blank lines, the order of things and lines
/// Softcore doesn't know all stay as they are; only the line for the key changes.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingsFile {
    pub path: PathBuf,
    lines: Vec<String>,
}

impl SettingsFile {
    /// The file as it is now (empty when there is none yet).
    pub fn load(path: &Path) -> std::io::Result<SettingsFile> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(SettingsFile::from_text(path, &text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SettingsFile { path: path.to_path_buf(), lines: Vec::new() }),
            Err(e) => Err(e),
        }
    }

    pub fn from_text(path: &Path, text: &str) -> SettingsFile {
        SettingsFile { path: path.to_path_buf(), lines: text.lines().map(String::from).collect() }
    }

    pub fn text(&self) -> String {
        let mut t = self.lines.join("\n");
        if !t.is_empty() {
            t.push('\n');
        }
        t
    }

    /// Write it back, all at once (a reader never sees half a file).
    pub fn save(&self) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = PathBuf::from(format!("{}.tmp", self.path.display()));
        std::fs::write(&tmp, self.text())?;
        std::fs::rename(&tmp, &self.path)
    }

    fn positions(&self, key: &str) -> Vec<usize> {
        let want = canon(key);
        self.lines.iter().enumerate().filter(|(_, l)| entry(l).is_some_and(|(k, _)| canon(k) == want)).map(|(i, _)| i).collect()
    }

    /// What this file says for a key (its last line for it), if it says anything.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.positions(key).last().and_then(|i| entry(&self.lines[*i])).map(|(_, v)| v)
    }

    /// Every line whose key starts with `prefix` (`bind.`, `output.`, `env.`), in order, the
    /// last one for each key.
    pub fn entries(&self, prefix: &str) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        for (k, v) in self.lines.iter().filter_map(|l| entry(l)).filter(|(k, _)| k.starts_with(prefix)) {
            out.retain(|(o, _)| canon(o) != canon(k));
            out.push((k.to_string(), v.to_string()));
        }
        out
    }

    /// Give a key a value. Its line changes where it stands; a key the file doesn't have yet
    /// goes at the end. Where an `include` comes after the key's line, the new line goes at
    /// the end too, since the included file would otherwise have the last word.
    pub fn set(&mut self, key: &str, value: &str) {
        let line = format!("{key}={value}");
        let at = self.positions(key);
        let last_include = self.lines.iter().rposition(|l| entry(l).is_some_and(|(k, _)| k == "include"));
        match at.last() {
            Some(&i) if last_include.is_none_or(|inc| inc < i) => {
                self.lines[i] = line;
                // Earlier lines for the key said nothing any more; a file with one is clearer.
                for &j in at[..at.len() - 1].iter().rev() {
                    self.lines.remove(j);
                }
            }
            _ => {
                for &j in at.iter().rev() {
                    self.lines.remove(j);
                }
                self.lines.push(line);
            }
        }
    }

    /// Take a key out, so its default (or an included file) applies. True when it was there.
    pub fn unset(&mut self, key: &str) -> bool {
        let at = self.positions(key);
        for &j in at.iter().rev() {
            self.lines.remove(j);
        }
        !at.is_empty()
    }

    /// The values of a key that is given many times (`spawn`), in order.
    pub fn list(&self, key: &str) -> Vec<String> {
        self.positions(key).iter().filter_map(|i| entry(&self.lines[*i])).map(|(_, v)| v.to_string()).collect()
    }

    /// Replace every line of such a key, where the first one stood.
    pub fn set_list(&mut self, key: &str, values: &[String]) {
        let at = self.positions(key);
        let first = at.first().copied().unwrap_or(self.lines.len());
        for &j in at.iter().rev() {
            self.lines.remove(j);
        }
        for (n, v) in values.iter().enumerate() {
            self.lines.insert(first + n, format!("{key}={v}"));
        }
    }
}
