use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    #[serde(default = "default_height")]
    pub height: u32,
    #[serde(default = "default_width")]
    pub width: u32,
    #[serde(default = "default_monitor_width")]
    pub monitor_width: u32,
    #[serde(default = "default_monitor_height")]
    pub monitor_height: u32,
}

fn default_height() -> u32 {
    440
}
fn default_width() -> u32 {
    960
}
fn default_monitor_width() -> u32 {
    1920
}
fn default_monitor_height() -> u32 {
    440
}

impl Default for Config {
    fn default() -> Self {
        Self {
            height: default_height(),
            width: default_width(),
            monitor_width: default_monitor_width(),
            monitor_height: default_monitor_height(),
        }
    }
}

pub fn user_config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("lingua")
}

pub fn pulse_config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("pulse")
}

pub fn user_config_path() -> PathBuf {
    user_config_dir().join("config.toml")
}

pub fn autostart_inited_path() -> PathBuf {
    user_config_dir().join("autostart-inited")
}

pub fn load_config() -> Result<Config, String> {
    let path = user_config_path();
    if !path.is_file() {
        let cfg = Config::default();
        save_config(&cfg)?;
        return Ok(cfg);
    }
    let raw = fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    toml::from_str(&raw).map_err(|e| format!("parse {}: {e}", path.display()))
}

pub fn save_config(cfg: &Config) -> Result<(), String> {
    let dir = user_config_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let raw = toml::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    fs::write(user_config_path(), raw).map_err(|e| format!("write {}: {e}", user_config_path().display()))?;
    Ok(())
}
