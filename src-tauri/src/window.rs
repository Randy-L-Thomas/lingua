use crate::config::Config;
use crate::ui_state::{UiState, WIN_MIN_H, WIN_MIN_W};
use tauri::{PhysicalPosition, PhysicalSize, Size, WebviewWindow};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidthMode {
    Half,
    Custom,
}

impl WidthMode {
    pub fn as_str(self) -> &'static str {
        match self {
            WidthMode::Half => "half",
            WidthMode::Custom => "custom",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "custom" => WidthMode::Custom,
            _ => WidthMode::Half,
        }
    }
}

/// One connected display, in the same physical pixels Tauri uses for set_position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MonitorGeom {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

pub fn dock_and_size(window: &WebviewWindow, cfg: &Config) -> Result<(), String> {
    let (origin, max_w) = strip_origin(window, cfg);
    let width = cfg.width.min(max_w);
    let x = origin.0 + (max_w as i32 - width as i32);
    let _ = window.set_min_size(Some(Size::Physical(PhysicalSize {
        width: WIN_MIN_W.min(max_w),
        height: WIN_MIN_H,
    })));
    let _ = window.set_max_size(None::<Size>);
    window
        .set_position(tauri::Position::Physical(PhysicalPosition { x, y: origin.1 }))
        .map_err(|e| e.to_string())?;
    window
        .set_size(Size::Physical(PhysicalSize {
            width,
            height: cfg.height,
        }))
        .map_err(|e| e.to_string())?;
    let _ = window.unminimize();
    let _ = window.set_focus();
    Ok(())
}

pub fn apply_saved(window: &WebviewWindow, cfg: &Config, ui: &UiState) -> Result<WidthMode, String> {
    let monitors = collect_monitors(window);
    let mode = resolve_saved_mode(ui, &monitors);
    if mode == WidthMode::Custom {
        apply_frame(window, ui.win_x, ui.win_y, ui.win_w, ui.win_h)?;
        return Ok(WidthMode::Custom);
    }
    dock_and_size(window, cfg)?;
    Ok(WidthMode::Half)
}

pub fn apply_frame(window: &WebviewWindow, x: i32, y: i32, w: u32, h: u32) -> Result<(), String> {
    let w = w.max(WIN_MIN_W);
    let h = h.max(WIN_MIN_H);
    let _ = window.set_min_size(Some(Size::Physical(PhysicalSize {
        width: WIN_MIN_W,
        height: WIN_MIN_H,
    })));
    let _ = window.set_max_size(None::<Size>);
    window
        .set_position(tauri::Position::Physical(PhysicalPosition { x, y }))
        .map_err(|e| e.to_string())?;
    window
        .set_size(Size::Physical(PhysicalSize { width: w, height: h }))
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn remember_frame(ui: &mut UiState, mode: WidthMode, x: i32, y: i32, w: u32, h: u32) {
    ui.win_mode = mode.as_str().into();
    ui.win_x = x;
    ui.win_y = y;
    ui.win_w = w;
    ui.win_h = h;
}

pub fn read_frame(window: &WebviewWindow) -> Option<(i32, i32, u32, u32)> {
    let pos = window.outer_position().ok()?;
    let size = window.outer_size().ok()?;
    Some((pos.x, pos.y, size.width, size.height))
}

pub fn frame_is_sane(w: u32, h: u32) -> bool {
    w >= WIN_MIN_W && h >= WIN_MIN_H && w <= 8000 && h <= 5000
}

/// True when the titlebar-ish corner of the frame sits on a connected monitor.
pub fn frame_visible_on(x: i32, y: i32, w: u32, h: u32, monitors: &[MonitorGeom]) -> bool {
    const MIN: i64 = 48;
    monitors.iter().any(|m| {
        let x0 = i64::from(x).max(i64::from(m.x));
        let y0 = i64::from(y).max(i64::from(m.y));
        let x1 = (i64::from(x) + i64::from(w)).min(i64::from(m.x) + i64::from(m.width));
        let y1 = (i64::from(y) + i64::from(h)).min(i64::from(m.y) + i64::from(m.height));
        x1 - x0 >= MIN && y1 - y0 >= MIN
    })
}

/// Custom coords from a disconnected bar are discarded so Lingua docks on a live display.
pub fn resolve_saved_mode(ui: &UiState, monitors: &[MonitorGeom]) -> WidthMode {
    let mode = WidthMode::parse(&ui.win_mode);
    if mode == WidthMode::Custom
        && frame_is_sane(ui.win_w, ui.win_h)
        && frame_visible_on(ui.win_x, ui.win_y, ui.win_w, ui.win_h, monitors)
    {
        WidthMode::Custom
    } else {
        WidthMode::Half
    }
}

pub fn pick_strip_origin(
    monitors: &[MonitorGeom],
    monitor_width: u32,
    bar_h_max: u32,
    width: u32,
    full_width: u32,
) -> ((i32, i32), u32) {
    let mut bar: Option<&MonitorGeom> = None;
    for m in monitors {
        if m.width != monitor_width || m.height > bar_h_max {
            continue;
        }
        let better = match bar {
            None => true,
            Some(b) => m.height <= b.height,
        };
        if better {
            bar = Some(m);
        }
    }
    if let Some(m) = bar {
        return ((m.x, m.y), m.width);
    }
    let work = monitors.iter().max_by_key(|m| {
        (
            u64::from(m.width).saturating_mul(u64::from(m.height)),
            m.width,
            m.height,
        )
    });
    if let Some(m) = work {
        return ((m.x, m.y), m.width.max(width));
    }
    ((0, 0), full_width)
}

pub fn strip_origin(window: &WebviewWindow, cfg: &Config) -> ((i32, i32), u32) {
    let monitors = collect_monitors(window);
    pick_strip_origin(
        &monitors,
        cfg.monitor_width,
        cfg.monitor_height.max(500),
        cfg.width,
        cfg.monitor_width,
    )
}

fn collect_monitors(window: &WebviewWindow) -> Vec<MonitorGeom> {
    window
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|mon| {
            let pos = mon.position();
            let size = mon.size();
            MonitorGeom {
                x: pos.x,
                y: pos.y,
                width: size.width,
                height: size.height,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        frame_is_sane, frame_visible_on, pick_strip_origin, remember_frame, resolve_saved_mode,
        MonitorGeom, WidthMode,
    };
    use crate::ui_state::UiState;

    fn tv() -> MonitorGeom {
        MonitorGeom {
            x: 0,
            y: 0,
            width: 2560,
            height: 1440,
        }
    }

    fn bar_below_tv() -> MonitorGeom {
        MonitorGeom {
            x: 0,
            y: 2160,
            width: 1920,
            height: 440,
        }
    }

    fn bar_at_saved_frame() -> MonitorGeom {
        MonitorGeom {
            x: 0,
            y: 2480,
            width: 1920,
            height: 450,
        }
    }

    #[test]
    fn remembers_custom_frame() {
        let mut ui = UiState::default();
        remember_frame(&mut ui, WidthMode::Custom, 10, 20, 800, 360);
        assert_eq!(ui.win_mode, "custom");
        assert_eq!((ui.win_x, ui.win_y, ui.win_w, ui.win_h), (10, 20, 800, 360));
        assert!(frame_is_sane(ui.win_w, ui.win_h));
        assert!(!frame_is_sane(100, 50));
    }

    #[test]
    fn prefers_short_bar_when_it_is_connected() {
        let origin = pick_strip_origin(&[tv(), bar_below_tv()], 1920, 500, 960, 1920);
        assert_eq!(origin, ((0, 2160), 1920));
    }

    #[test]
    fn docks_to_tv_when_bar_is_unplugged() {
        let origin = pick_strip_origin(&[tv()], 1920, 500, 960, 1920);
        assert_eq!(origin, ((0, 0), 2560));
    }

    #[test]
    fn saved_bar_frame_is_off_the_tv() {
        assert!(!frame_visible_on(779, 2524, 1920, 440, &[tv()]));
        assert!(frame_visible_on(
            779,
            2524,
            1920,
            440,
            &[tv(), bar_at_saved_frame()]
        ));
        assert!(frame_visible_on(40, 20, 960, 440, &[tv()]));
    }

    #[test]
    fn offscreen_custom_falls_back_to_dock() {
        let ui = UiState {
            win_mode: "custom".into(),
            win_x: 779,
            win_y: 2524,
            win_w: 1920,
            win_h: 440,
            ..UiState::default()
        };
        assert_eq!(resolve_saved_mode(&ui, &[tv()]), WidthMode::Half);
        assert_eq!(
            resolve_saved_mode(&ui, &[tv(), bar_at_saved_frame()]),
            WidthMode::Custom
        );
    }

    #[test]
    fn dock_x_is_right_edge_minus_width() {
        let origin = pick_strip_origin(&[bar_below_tv()], 1920, 500, 960, 1920);
        let max_w = origin.1;
        let width = 960u32;
        let x = origin.0 .0 + (max_w as i32 - width as i32);
        assert_eq!(x, 960);
    }
}
