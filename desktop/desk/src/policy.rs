use crate::protocol::Access;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const BLOCKED: &[&str] = &[
    "com.apple.Terminal",
    "com.googlecode.iterm2",
    "com.mitchellh.ghostty",
    "dev.warp.Warp-Stable",
    "net.kovidgoyal.kitty",
    "org.alacritty",
    "com.github.wez.wezterm",
    "com.apple.keychainaccess",
    "com.apple.Passwords",
    "com.apple.systempreferences",
    "com.apple.SecurityAgent",
    "com.apple.ScriptEditor2",
    "com.apple.Automator",
    "com.1password.1password",
    "com.agilebits.onepassword7",
    "com.bitwarden.desktop",
    "com.lastpass.LastPass",
    "com.dashlane.dashlanephonefinal",
    "com.openai.codex",
    "com.openai.chat",
];

const BLOCKED_NAMES: &[&str] = &[
    "terminal",
    "iterm",
    "iterm2",
    "keychain access",
    "passwords",
    "system settings",
    "system preferences",
    "1password",
    "bitwarden",
    "lastpass",
    "dashlane",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub allow: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config { allow: vec!["com.apple.calculator".into(), "com.apple.TextEdit".into()] }
    }
}

pub fn path() -> PathBuf {
    std::env::var_os("DESK_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config/desk/config.json"))
}

pub fn load() -> Result<Config, String> {
    match std::fs::read_to_string(path()) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| format!("{}: {e}", path().display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(e.to_string()),
    }
}

pub fn save(c: &Config) -> Result<(), String> {
    let p = path();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(&p, serde_json::to_string_pretty(c).unwrap_or_default()).map_err(|e| e.to_string())
}

pub fn access(c: &Config, name: &str, bundle: Option<&str>) -> Access {
    let hit = |s: &str| s.eq_ignore_ascii_case(name) || bundle.is_some_and(|b| s.eq_ignore_ascii_case(b));
    if BLOCKED.iter().any(|s| hit(s)) || BLOCKED_NAMES.iter().any(|s| s.eq_ignore_ascii_case(name)) {
        Access::Blocked
    } else if c.allow.iter().any(|s| hit(s)) {
        Access::Allowed
    } else {
        Access::NotListed
    }
}
