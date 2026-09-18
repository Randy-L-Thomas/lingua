use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WebviewWindow};

pub fn restore_main(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

pub fn hide_to_tray(app: &AppHandle) -> Result<(), String> {
    set_visible(app, true);
    let win = app.get_webview_window("main").ok_or("no window")?;
    win.hide().map_err(|e| e.to_string())
}

pub fn apply_launch_hide(win: &WebviewWindow, app: &AppHandle, mode: &str) {
    match mode {
        "min" => {
            set_visible(app, false);
            let _ = win.minimize();
        }
        "tray" => {
            set_visible(app, true);
            let _ = win.hide();
        }
        _ => {
            set_visible(app, false);
        }
    }
}

pub fn set_visible(app: &AppHandle, on: bool) {
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_visible(on);
    }
}

pub fn install(app: &tauri::App, tooltip: &str) -> Result<(), Box<dyn std::error::Error>> {
    let show = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let icon = app.default_window_icon().cloned().ok_or("no window icon")?;
    TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip(tooltip)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => restore_main(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                restore_main(tray.app_handle());
            }
        })
        .build(app)?;
    set_visible(app.handle(), false);
    Ok(())
}
