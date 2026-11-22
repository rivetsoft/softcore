//! `softcore settings`: read and change the settings file from a terminal, the way a settings
//! window does (one line changed, the rest left alone). A running Softcore picks changes up
//! by itself.

use serde_json::json;
use softcore_settings::{Config, Group, Kind, SETTINGS, SettingsFile, check, config_file, parse_bind, parse_combo, parse_output, setting};

const HELP: &str = "\
softcore settings              Every setting and its value now
softcore settings get KEY      One value
softcore settings set KEY VAL  Change one (checked first); the rest of the file stays as it is
softcore settings unset KEY    Back to the default
softcore settings describe     What each setting is, as JSON (type, range, default, label)

Keys there are many of are named: bind.Mod+T, output.DP-5, env.NAME.
";

fn kind_json(k: Kind) -> serde_json::Value {
    match k {
        Kind::Int { min, max } => json!({"type": "int", "min": min, "max": max}),
        Kind::OptInt { min, max } => json!({"type": "int", "min": min, "max": max, "optional": true}),
        Kind::Float { min, max, step } => json!({"type": "float", "min": min, "max": max, "step": step}),
        Kind::Bool => json!({"type": "bool"}),
        Kind::Color => json!({"type": "color"}),
        Kind::OptColor => json!({"type": "color", "optional": true}),
        Kind::Choice(words) => json!({"type": "choice", "choices": words}),
        Kind::Text => json!({"type": "text"}),
        Kind::Widths => json!({"type": "widths"}),
    }
}

/// Whether a line would be understood, before it is written.
fn valid(key: &str, value: &str) -> Result<(), String> {
    if let Some(s) = setting(key) {
        return check(s, value);
    }
    if let Some(combo) = key.strip_prefix("bind.") {
        let (_, k) = parse_combo(combo)?;
        if crate::input::key_sym(&k).is_none() {
            return Err(format!("there is no key called {k:?}"));
        }
        return if value.is_empty() { Ok(()) } else { parse_bind(combo, value).map(|_| ()) };
    }
    if let Some(name) = key.strip_prefix("output.") {
        return parse_output(name, value).map(|_| ());
    }
    if key.starts_with("env.") {
        return Ok(());
    }
    Err(format!("softcore has no setting called {key:?} (softcore settings lists them)"))
}

pub fn run(args: &[String]) -> i32 {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let path = config_file();
    let open = || SettingsFile::load(&path).map_err(|e| format!("couldn't read {}: {e}", path.display()));
    let done = |r: Result<(), String>| match r {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("softcore settings: {e}");
            1
        }
    };
    match words.as_slice() {
        [] => {
            let c = Config::load();
            let file = open().ok();
            println!("# {}", path.display());
            for g in Group::ALL {
                println!("\n# {}", g.label());
                for s in SETTINGS.iter().filter(|s| s.group == g) {
                    // A star: set in the file, not the default.
                    let own = file.as_ref().is_some_and(|f| f.get(s.key).is_some());
                    println!("{}{:<40} # {}", if own { "*" } else { " " }, format!("{}={}", s.key, c.value(s.key).unwrap_or("")), s.label);
                }
            }
            if let Some(f) = &file {
                for prefix in ["bind.", "output.", "env."] {
                    for (k, v) in f.entries(prefix) {
                        println!("*{k}={v}");
                    }
                }
                for v in f.list("spawn") {
                    println!("*spawn={v}");
                }
            }
            for e in &c.errors {
                eprintln!("softcore settings: {e}");
            }
            0
        }
        ["get", key] => match setting(key) {
            Some(_) => {
                println!("{}", Config::load().value(key).unwrap_or(""));
                0
            }
            None => done(open().and_then(|f| f.get(key).map(|v| println!("{v}")).ok_or(format!("{key} isn't set")))),
        },
        ["set", key, value @ ..] => {
            let value = value.join(" ");
            done(valid(key, &value).and_then(|()| open()).and_then(|mut f| {
                f.set(key, &value);
                f.save().map_err(|e| format!("couldn't write {}: {e}", path.display()))
            }))
        }
        ["unset", key] => done(open().and_then(|mut f| {
            f.unset(key);
            f.save().map_err(|e| format!("couldn't write {}: {e}", path.display()))
        })),
        ["describe"] => {
            let all: Vec<_> = SETTINGS.iter().map(|s| json!({"key": s.key, "group": s.group.label(), "label": s.label, "help": s.help, "default": s.default, "kind": kind_json(s.kind)})).collect();
            println!("{}", serde_json::to_string_pretty(&json!({"file": path, "settings": all, "keyed": ["bind.KEYS=ACTION", "output.NAME=mode=WxH@HZ scale=N position=X,Y | off", "env.NAME=VALUE"], "lists": ["spawn"]})).unwrap_or_default());
            0
        }
        _ => {
            eprint!("{HELP}");
            2
        }
    }
}
