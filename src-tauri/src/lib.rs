mod chat_store;
mod config;
mod dsf;
mod mt_lex;
mod mt_opus;
mod mt_seq2seq;
mod ocr_text;
mod ollama;
mod translate;
mod ui_state;
mod window;
mod winui;

use config::Config;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{Emitter, Manager};
use ui_state::UiState;
use window::WidthMode;

struct AppState {
    cfg: Mutex<Config>,
    ollama: reqwest::Client,
    dsf: reqwest::Client,
    width: Mutex<WidthMode>,
    ui: Mutex<UiState>,
    geom_lock: Mutex<bool>,
}

#[tauri::command]
fn set_width_mode(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    mode: String,
) -> Result<(), String> {
    let win = app.get_webview_window("main").ok_or("no window")?;
    let cfg = state.cfg.lock().unwrap().clone();
    let m = if mode == "custom" {
        WidthMode::Custom
    } else {
        WidthMode::Half
    };
    *state.geom_lock.lock().unwrap() = true;
    *state.width.lock().unwrap() = m;
    if m == WidthMode::Custom {
        let ui = state.ui.lock().unwrap().clone();
        window::apply_frame(&win, ui.win_x, ui.win_y, ui.win_w, ui.win_h)?;
    } else {
        window::dock_and_size(&win, &cfg)?;
    }
    if let Some((x, y, w, h)) = window::read_frame(&win) {
        let mut ui = state.ui.lock().unwrap();
        window::remember_frame(&mut ui, m, x, y, w, h);
        let _ = ui_state::save(&ui);
    }
    let _ = app.emit("width-mode", m.as_str());
    let unlock = Arc::clone(&state);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(250));
        *unlock.geom_lock.lock().unwrap() = false;
    });
    Ok(())
}

#[tauri::command]
fn get_settings(state: tauri::State<'_, Arc<AppState>>) -> Config {
    state.cfg.lock().unwrap().clone()
}

#[tauri::command]
fn save_settings(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    cfg: Config,
) -> Result<String, String> {
    config::save_config(&cfg)?;
    *state.cfg.lock().unwrap() = cfg.clone();
    let mode = *state.width.lock().unwrap();
    if let Some(win) = app.get_webview_window("main") {
        if mode == WidthMode::Custom {
            let ui = state.ui.lock().unwrap().clone();
            let _ = window::apply_saved(&win, &cfg, &ui);
        } else {
            let _ = window::dock_and_size(&win, &cfg);
        }
    }
    Ok(format!("saved {}", config::user_config_path().display()))
}

#[tauri::command]
fn app_meta() -> serde_json::Value {
    serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "config_path": config::user_config_path().display().to_string(),
    })
}

#[tauri::command]
fn get_ui(state: tauri::State<'_, Arc<AppState>>) -> UiState {
    state.ui.lock().unwrap().clone()
}

#[tauri::command]
fn save_ui(state: tauri::State<'_, Arc<AppState>>, mut ui: UiState) -> Result<(), String> {
    let mut guard = state.ui.lock().unwrap();
    ui.font_px = ui_state::clamp_font_px(ui.font_px);
    ui.win_mode = guard.win_mode.clone();
    ui.win_x = guard.win_x;
    ui.win_y = guard.win_y;
    ui.win_w = guard.win_w;
    ui.win_h = guard.win_h;
    ui_state::save(&ui)?;
    *guard = ui;
    Ok(())
}

#[tauri::command]
fn set_font_px(state: tauri::State<'_, Arc<AppState>>, px: u32) -> Result<u32, String> {
    let next = ui_state::clamp_font_px(px);
    let mut ui = state.ui.lock().unwrap();
    ui.font_px = next;
    ui_state::save(&ui)?;
    Ok(next)
}

#[tauri::command]
fn list_app_windows() -> Result<Vec<winui::WindowInfo>, String> {
    winui::list_windows()
}

#[tauri::command]
async fn translate_text(
    app: tauri::AppHandle,
    source: String,
    from: String,
    to: String,
    llm: Option<bool>,
) -> Result<translate::TranslateOut, String> {
    let on_progress = {
        let app = app.clone();
        Some(std::sync::Arc::new(move |msg: &str| {
            let _ = app.emit("mt-progress", msg);
        }) as translate::ProgressFn)
    };
    translate::translate(
        &from,
        &to,
        &source,
        llm.unwrap_or(translate::default_use_opus()),
        on_progress,
    )
    .await
}

#[tauri::command]
fn detect_mt_lang(text: String) -> Option<String> {
    translate::detect_lang_confident(&text).map(|s| s.to_string())
}

#[tauri::command]
async fn capture_ocr(
    state: tauri::State<'_, Arc<AppState>>,
    title: Option<String>,
) -> Result<winui::OcrOut, String> {
    let saved = state.ui.lock().unwrap().wa_title.clone();
    let title = title.unwrap_or_default();
    let title = if title.trim().is_empty() {
        saved
    } else {
        title
    };
    tokio::task::spawn_blocking(move || winui::capture_and_ocr(&title))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn ollama_models(state: tauri::State<'_, Arc<AppState>>) -> Result<Vec<String>, String> {
    let url = state.ui.lock().unwrap().ollama_url.clone();
    ollama::list_models(&state.ollama, &url).await
}

#[tauri::command]
async fn ollama_chat(
    state: tauri::State<'_, Arc<AppState>>,
    model: String,
    messages: Vec<ollama::ChatMsg>,
) -> Result<String, String> {
    let url = state.ui.lock().unwrap().ollama_url.clone();
    ollama::chat(&state.ollama, &url, &model, messages).await
}

#[tauri::command]
fn chat_load() -> chat_store::ChatLog {
    chat_store::load()
}

#[tauri::command]
fn chat_save(log: chat_store::ChatLog) -> Result<(), String> {
    chat_store::save(&log)
}

#[tauri::command]
fn chat_clear() -> Result<(), String> {
    chat_store::save(&chat_store::ChatLog::default())
}

/// Deployments reachable through the DSF allowlist. No HTTP: the deployment name
/// is configuration, so listing it must not cost a completion.
#[tauri::command]
fn dsf_models() -> Vec<String> {
    vec![dsf::deployment_from_env()]
}

#[tauri::command]
async fn dsf_ping(state: tauri::State<'_, Arc<AppState>>) -> Result<String, String> {
    let key = dsf::key_from_env()?;
    dsf::ping(
        &state.dsf,
        &dsf::base_from_env(),
        &key,
        &dsf::deployment_from_env(),
    )
    .await
}

#[tauri::command]
async fn dsf_chat(
    state: tauri::State<'_, Arc<AppState>>,
    model: String,
    messages: Vec<ollama::ChatMsg>,
) -> Result<String, String> {
    let key = dsf::key_from_env()?;
    let model = if model.trim().is_empty() {
        dsf::deployment_from_env()
    } else {
        model
    };
    dsf::chat(&state.dsf, &dsf::base_from_env(), &key, &model, messages).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let cfg = config::load_config().unwrap_or_else(|e| {
        eprintln!("lingua: {e}");
        std::process::exit(1);
    });
    let ollama = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .connect_timeout(Duration::from_secs(3))
        .no_proxy()
        .build()
        .expect("ollama client");
    let dsf = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .connect_timeout(Duration::from_secs(5))
        .no_proxy()
        .build()
        .expect("dsf client");
    let state = Arc::new(AppState {
        cfg: Mutex::new(cfg),
        ollama,
        dsf,
        width: Mutex::new(WidthMode::Half),
        ui: Mutex::new(ui_state::load()),
        geom_lock: Mutex::new(false),
    });

    tauri::Builder::default()
        .plugin({
            let si_state = state.clone();
            tauri_plugin_single_instance::init(move |app, _argv, _cwd| {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.unminimize();
                    let _ = win.set_focus();
                    let pinned = si_state.ui.lock().unwrap().pinned;
                    let _ = win.set_always_on_top(pinned);
                }
            })
        })
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .manage(state.clone())
        .setup(move |app| {
            ensure_default_autostart(app.handle());
            winui::start_focus_watch();
            let win = app.get_webview_window("main").expect("main window");
            let cfg = state.cfg.lock().unwrap().clone();
            let saved = state.ui.lock().unwrap().clone();
            *state.geom_lock.lock().unwrap() = true;
            let mode = window::apply_saved(&win, &cfg, &saved).unwrap_or(WidthMode::Half);
            let _ = win.set_always_on_top(saved.pinned);
            *state.width.lock().unwrap() = mode;
            let _ = app.handle().emit("width-mode", mode.as_str());
            let unlock = state.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(250));
                *unlock.geom_lock.lock().unwrap() = false;
            });
            let geom_win = win.clone();
            let loop_state = state.clone();
            win.on_window_event(move |ev| {
                if !matches!(
                    ev,
                    tauri::WindowEvent::Resized(_) | tauri::WindowEvent::Moved(_)
                ) {
                    return;
                }
                if *loop_state.geom_lock.lock().unwrap() {
                    return;
                }
                let Some((x, y, w, h)) = window::read_frame(&geom_win) else {
                    return;
                };
                if !window::frame_is_sane(w, h) {
                    return;
                }
                let mut width = loop_state.width.lock().unwrap();
                if !matches!(*width, WidthMode::Custom) {
                    *width = WidthMode::Custom;
                    let _ = geom_win.emit("width-mode", "custom");
                }
                let mut ui = loop_state.ui.lock().unwrap();
                window::remember_frame(&mut ui, WidthMode::Custom, x, y, w, h);
                let _ = ui_state::save(&ui);
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            set_width_mode,
            get_settings,
            save_settings,
            app_meta,
            get_ui,
            save_ui,
            set_font_px,
            list_app_windows,
            translate_text,
            detect_mt_lang,
            capture_ocr,
            ollama_models,
            ollama_chat,
            dsf_models,
            dsf_ping,
            dsf_chat,
            chat_load,
            chat_save,
            chat_clear
        ])
        .run(tauri::generate_context!())
        .expect("error while running lingua");
}

fn ensure_default_autostart(app: &tauri::AppHandle) {
    use tauri_plugin_autostart::ManagerExt;
    let marker = config::autostart_inited_path();
    if marker.is_file() {
        return;
    }
    if let Some(dir) = marker.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = app.autolaunch().enable();
    let _ = std::fs::write(&marker, b"1\n");
}
