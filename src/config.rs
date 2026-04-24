use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub defaults: Defaults,
    pub capture: CaptureCfg,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Defaults {
    pub counter_radius: f32,
    pub pixelate_block: u32,
    pub save_dir: String,
    pub filename_pattern: String,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            counter_radius: 16.0,
            pixelate_block: 10,
            save_dir: "~/Pictures/screenshots".into(),
            filename_pattern: "%Y%m%d-%H%M%S.png".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CaptureCfg {
    pub include_cursor: bool,
}

impl Config {
    pub fn load_or_default() -> Self {
        let path = config_path();
        match std::fs::read_to_string(&path) {
            Ok(s) => match toml::from_str::<Self>(&s) {
                Ok(c) => {
                    tracing::info!(path = %path.display(), "loaded config");
                    c
                }
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "invalid config; using defaults");
                    Self::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::info!(path = %path.display(), "no config file; using defaults");
                Self::default()
            }
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "config read error; using defaults");
                Self::default()
            }
        }
    }
}

pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("rustshot")
        .join("config.toml")
}

/// Build an auto-save path from `save_dir` (with `~` expanded) and a strftime-style filename pattern.
pub fn auto_save_path(save_dir: &str, pattern: &str) -> PathBuf {
    let dir = expand_tilde(save_dir);
    let now = chrono::Local::now();
    let fname = now.format(pattern).to_string();
    dir.join(fname)
}

fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    if p == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    PathBuf::from(p)
}
