use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiState {
    #[serde(default = "default_module")]
    pub last_module: String,
    #[serde(default = "default_wa")]
    pub wa_title: String,
    #[serde(default = "default_from")]
    pub mt_from: String,
    #[serde(default = "default_to")]
    pub mt_to: String,
    #[serde(default)]
    pub mt_enrich: bool,
    #[serde(default)]
    pub ollama_model: String,
    /// "ollama" or "dsf". Unknown values fall back to ollama.
    #[serde(default = "default_chat_engine")]
    pub chat_engine: String,
    #[serde(default = "default_ollama")]
    pub ollama_url: String,
    #[serde(default = "default_font_px")]
    pub font_px: u32,
    #[serde(default = "default_win_mode")]
    pub win_mode: String,
    #[serde(default)]
    pub win_x: i32,
    #[serde(default)]
    pub win_y: i32,
    #[serde(default)]
    pub win_w: u32,
    #[serde(default)]
    pub win_h: u32,
    /// Missing field must stay pinned so existing ui.json does not unpin on upgrade.
    #[serde(default = "default_pinned")]
    pub pinned: bool,
    /// "off" | "min" | "tray". Missing stays visible on launch.
    #[serde(default = "default_launch_hide")]
    pub launch_hide: String,
}

fn default_module() -> String {
    "translate".into()
}
fn default_wa() -> String {
    "WhatsApp".into()
}
fn default_from() -> String {
    "es".into()
}
fn default_to() -> String {
    "en".into()
}
fn default_chat_engine() -> String {
    "ollama".into()
}
fn default_ollama() -> String {
    "http://127.0.0.1:11434".into()
}
fn default_font_px() -> u32 {
    13
}
fn default_win_mode() -> String {
    "half".into()
}
fn default_pinned() -> bool {
    true
}
fn default_launch_hide() -> String {
    "off".into()
}

pub fn normalize_launch_hide(s: &str) -> String {
    match s {
        "min" | "tray" => s.into(),
        _ => default_launch_hide(),
    }
}

pub const WIN_MIN_W: u32 = 480;
pub const WIN_MIN_H: u32 = 200;

pub const FONT_PX_MIN: u32 = 11;
pub const FONT_PX_MAX: u32 = 22;

pub fn clamp_font_px(px: u32) -> u32 {
    px.clamp(FONT_PX_MIN, FONT_PX_MAX)
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            last_module: default_module(),
            wa_title: default_wa(),
            mt_from: default_from(),
            mt_to: default_to(),
            mt_enrich: false,
            ollama_model: String::new(),
            chat_engine: default_chat_engine(),
            ollama_url: default_ollama(),
            font_px: default_font_px(),
            win_mode: default_win_mode(),
            win_x: 0,
            win_y: 0,
            win_w: 0,
            win_h: 0,
            pinned: default_pinned(),
            launch_hide: default_launch_hide(),
        }
    }
}

fn path() -> PathBuf {
    crate::config::user_config_dir().join("ui.json")
}

fn seed_from_pulse() -> Option<UiState> {
    let p = crate::config::pulse_config_dir().join("ui.json");
    let raw = fs::read_to_string(&p).ok()?;
    let mut state: UiState = serde_json::from_str(&raw).ok()?;
    state.font_px = clamp_font_px(state.font_px);
    // Do not reuse Pulse's left-half frame; Lingua docks on the right.
    state.win_mode = default_win_mode();
    state.win_x = 0;
    state.win_y = 0;
    state.win_w = 0;
    state.win_h = 0;
    // First visible run must not inherit Pulse hide-on-launch.
    state.launch_hide = default_launch_hide();
    Some(state)
}

pub fn load() -> UiState {
    let p = path();
    if let Ok(raw) = fs::read_to_string(&p) {
        let mut state: UiState = serde_json::from_str(&raw).unwrap_or_default();
        state.font_px = clamp_font_px(state.font_px);
        state.launch_hide = normalize_launch_hide(&state.launch_hide);
        return state;
    }
    seed_from_pulse().unwrap_or_default()
}

pub fn save(state: &UiState) -> Result<(), String> {
    let dir = crate::config::user_config_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let raw = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
    fs::write(path(), raw).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::{default_launch_hide, UiState};

    #[test]
    fn missing_pinned_deserializes_as_true() {
        let ui: UiState = serde_json::from_str(r#"{"last_module":"ocr"}"#).unwrap();
        assert!(ui.pinned, "upgrade without pinned must stay always-on-top");
        assert_eq!(ui.last_module, "ocr");
    }

    #[test]
    fn explicit_pinned_false_stays_unpinned() {
        let ui: UiState = serde_json::from_str(r#"{"pinned":false}"#).unwrap();
        assert!(!ui.pinned);
    }

    #[test]
    fn missing_chat_engine_defaults_to_ollama() {
        let ui: UiState = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(
            ui.chat_engine, "ollama",
            "upgrade must not silently switch to a paid engine"
        );
    }

    #[test]
    fn saved_chat_engine_round_trips() {
        let ui: UiState = serde_json::from_str(r#"{"chat_engine":"dsf"}"#).unwrap();
        assert_eq!(ui.chat_engine, "dsf");
    }

    #[test]
    fn default_ui_is_pinned() {
        assert!(UiState::default().pinned);
    }

    #[test]
    fn missing_launch_hide_is_off() {
        let ui: UiState = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(ui.launch_hide, "off");
    }

    #[test]
    fn seed_from_pulse_does_not_inherit_hide() {
        let mut ui: UiState = serde_json::from_str(r#"{"launch_hide":"tray"}"#).unwrap();
        ui.launch_hide = default_launch_hide();
        assert_eq!(ui.launch_hide, "off");
    }
}
