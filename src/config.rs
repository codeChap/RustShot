use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub defaults: Defaults,
    pub capture: CaptureCfg,
    pub palette: Palette,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Defaults {
    pub color: String,
    pub width: f32,
    pub counter_radius: f32,
    pub blur_sigma: f32,
    pub initial_tool: String,
    pub save_dir: String,
    pub filename_pattern: String,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            color: "#ff3232".into(),
            width: 4.0,
            counter_radius: 16.0,
            blur_sigma: 12.0,
            initial_tool: "rect".into(),
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Palette {
    pub colors: Vec<String>,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            colors: vec![
                "#ff3232".into(),
                "#ffc800".into(),
                "#50c850".into(),
                "#32b4dc".into(),
                "#4664dc".into(),
                "#c850c8".into(),
                "#ffffff".into(),
                "#000000".into(),
            ],
        }
    }
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

pub fn parse_color(s: &str) -> Option<image::Rgba<u8>> {
    let s = s.strip_prefix('#').unwrap_or(s);
    match s.len() {
        6 => Some(image::Rgba([
            u8::from_str_radix(&s[0..2], 16).ok()?,
            u8::from_str_radix(&s[2..4], 16).ok()?,
            u8::from_str_radix(&s[4..6], 16).ok()?,
            255,
        ])),
        8 => Some(image::Rgba([
            u8::from_str_radix(&s[0..2], 16).ok()?,
            u8::from_str_radix(&s[2..4], 16).ok()?,
            u8::from_str_radix(&s[4..6], 16).ok()?,
            u8::from_str_radix(&s[6..8], 16).ok()?,
        ])),
        _ => None,
    }
}

pub fn parse_tool(s: &str) -> Option<crate::canvas::ToolKind> {
    use crate::canvas::ToolKind;
    match s.trim().to_ascii_lowercase().as_str() {
        "pencil" => Some(ToolKind::Pencil),
        "arrow" => Some(ToolKind::Arrow),
        "rect" | "rectangle" => Some(ToolKind::Rect),
        "ellipse" | "circle" => Some(ToolKind::Ellipse),
        "blur" => Some(ToolKind::Blur),
        "counter" | "marker" => Some(ToolKind::Counter),
        _ => None,
    }
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
