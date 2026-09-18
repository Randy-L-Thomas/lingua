//! Window pop, enumeration, capture, and WinRT OCR (Windows).

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct WindowInfo {
    pub title: String,
    pub hwnd: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct OcrOut {
    pub text: String,
    pub source: String,
}

/// True when the bitmap is a failed grab: no contrast (pure black, solid fill).
/// Dark WhatsApp still has message text, so luma range stays high.
pub(crate) fn is_blank_bgra(bgra: &[u8]) -> bool {
    if bgra.len() < 16 {
        return true;
    }
    let mut min = 255u32;
    let mut max = 0u32;
    let mut n = 0u32;
    for px in bgra.chunks_exact(4).step_by(8) {
        let y = (u32::from(px[0]) + u32::from(px[1]) + u32::from(px[2])) / 3;
        min = min.min(y);
        max = max.max(y);
        n += 1;
    }
    n == 0 || max.saturating_sub(min) < 10
}

pub(crate) fn require_ocr_text(text: String) -> Result<String, String> {
    if text.trim().is_empty() {
        Err("OCR found no text — is the chat visible?".into())
    } else {
        Ok(text)
    }
}

pub(crate) fn pick_largest_hwnd(candidates: &[(u64, i64)]) -> Option<u64> {
    candidates
        .iter()
        .filter(|(_, area)| *area > 0)
        .max_by_key(|(_, area)| *area)
        .map(|(hwnd, _)| *hwnd)
}

pub(crate) fn looks_like_whatsapp(title: &str, exe: &str) -> bool {
    title.to_ascii_lowercase().contains("whatsapp")
        || exe.to_ascii_lowercase().contains("whatsapp")
}

/// WhatsApp message pane: drop the left chat list, top chrome, and composer.
/// `(x, y, width, height)` in the full-window bitmap.
pub(crate) fn chat_pane_crop(w: u32, h: u32) -> (u32, u32, u32, u32) {
    if w < 200 || h < 160 {
        return (0, 0, w, h);
    }
    let left = if w >= 900 {
        (w * 30) / 100
    } else if w >= 700 {
        (w * 22) / 100
    } else {
        0
    };
    let top = ((h * 14) / 100).clamp(40, 96);
    let bottom = ((h * 12) / 100).clamp(36, 88);
    let x = left.min(w.saturating_sub(120));
    let y = top.min(h.saturating_sub(80));
    let cw = w.saturating_sub(x);
    let ch = h.saturating_sub(y + bottom).max(80);
    (x, y, cw, ch)
}

/// Pulse overlay only. Never park WhatsApp or any other app, even if PIDs collide.
pub(crate) fn should_park_overlay(
    hwnd_pid: u32,
    pulse_pid: u32,
    title: &str,
    exe: &str,
    overlaps_target: bool,
) -> bool {
    overlaps_target
        && hwnd_pid == pulse_pid
        && is_pulse_overlay(title, exe)
        && !looks_like_whatsapp(title, exe)
}

pub(crate) fn is_pulse_overlay(title: &str, exe: &str) -> bool {
    if looks_like_whatsapp(title, exe) {
        return false;
    }
    let t = title.to_ascii_lowercase();
    let e = exe.to_ascii_lowercase().replace('/', "\\");
    t.contains("pulse")
        || e.contains("pulse.exe")
        || t.contains("lingua")
        || e.contains("lingua.exe")
}

/// Pulse must never ShowWindow / hide / minimize / close the WhatsApp HWND.
pub(crate) fn may_show_window_on_target(title: &str, exe: &str) -> bool {
    !looks_like_whatsapp(title, exe)
}

/// Park far left of every monitor. Sliding right can land on WhatsApp's screen.
pub(crate) fn park_position(_left: i32, top: i32, _width: i32) -> (i32, i32) {
    (-32000, top)
}

pub(crate) fn crop_bgra(
    bgra: &[u8],
    w: u32,
    h: u32,
    x: u32,
    y: u32,
    cw: u32,
    ch: u32,
) -> (Vec<u8>, u32, u32) {
    let x = x.min(w.saturating_sub(1));
    let y = y.min(h.saturating_sub(1));
    let cw = cw.min(w.saturating_sub(x)).max(1);
    let ch = ch.min(h.saturating_sub(y)).max(1);
    let mut out = vec![0u8; cw as usize * ch as usize * 4];
    for row in 0..ch as usize {
        let si = ((y as usize + row) * w as usize + x as usize) * 4;
        let di = row * cw as usize * 4;
        out[di..di + cw as usize * 4].copy_from_slice(&bgra[si..si + cw as usize * 4]);
    }
    (out, cw, ch)
}

/// Paint a rectangle black so overlay chrome (Pulse) is not OCR-ed.
pub(crate) fn mask_bgra(bgra: &mut [u8], bw: u32, bh: u32, x: i32, y: i32, cw: i32, ch: i32) {
    if bw == 0 || bh == 0 || cw <= 0 || ch <= 0 {
        return;
    }
    let x0 = x.clamp(0, bw as i32) as u32;
    let y0 = y.clamp(0, bh as i32) as u32;
    let x1 = (x + cw).clamp(0, bw as i32) as u32;
    let y1 = (y + ch).clamp(0, bh as i32) as u32;
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    for row in y0..y1 {
        let start = (row * bw + x0) as usize * 4;
        let end = (row * bw + x1) as usize * 4;
        if end > bgra.len() {
            break;
        }
        for px in bgra[start..end].chunks_exact_mut(4) {
            px[0] = 0;
            px[1] = 0;
            px[2] = 0;
            px[3] = 255;
        }
    }
}

/// Nearest-neighbor shrink so WinRT OCR is not chewing a 4K desktop for 15s.
pub(crate) fn scale_bgra(bgra: Vec<u8>, w: u32, h: u32, max_w: u32) -> (Vec<u8>, u32, u32) {
    if w == 0 || h == 0 || w <= max_w {
        return (bgra, w, h);
    }
    let nw = max_w;
    let nh = ((u64::from(h) * u64::from(nw)) / u64::from(w)).max(1) as u32;
    let mut out = vec![0u8; nw as usize * nh as usize * 4];
    for y in 0..nh as usize {
        let sy = y * h as usize / nh as usize;
        for x in 0..nw as usize {
            let sx = x * w as usize / nw as usize;
            let si = (sy * w as usize + sx) * 4;
            let di = (y * nw as usize + x) * 4;
            out[di..di + 4].copy_from_slice(&bgra[si..si + 4]);
        }
    }
    (out, nw, nh)
}

#[cfg(not(windows))]
pub fn pop_cursor() -> Result<String, String> {
    Err("pop is Windows-only".into())
}

#[cfg(not(windows))]
pub fn list_windows() -> Result<Vec<WindowInfo>, String> {
    Err("Windows-only".into())
}

#[cfg(not(windows))]
pub fn capture_and_ocr(title_substr: &str) -> Result<OcrOut, String> {
    let _ = title_substr;
    Err("OCR is Windows-only".into())
}

#[cfg(not(windows))]
pub fn start_focus_watch() {}

#[cfg(windows)]
mod imp {
    use super::{
        chat_pane_crop, crop_bgra, is_blank_bgra, looks_like_whatsapp, may_show_window_on_target,
        park_position, pick_largest_hwnd, require_ocr_text, scale_bgra, should_park_overlay,
        OcrOut, WindowInfo,
    };
    use std::mem::size_of;
    use std::sync::Mutex;
    use std::time::Duration;
    use windows::core::{Interface, BOOL, PWSTR};
    use windows::Globalization::Language;
    use windows::Graphics::Imaging::BitmapDecoder;
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};
    use windows::Win32::Foundation::{CloseHandle, HMODULE, HWND, LPARAM, RECT};
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
        D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE,
        D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    };
    use windows::Win32::Graphics::Dwm::{
        DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
    };
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIAdapter, IDXGIFactory1, IDXGIOutput1, IDXGIResource,
        DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
    };
    use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
    use windows::Win32::System::Threading::{
        GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetAncestor, GetClientRect, GetForegroundWindow, GetWindowRect,
        GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible,
        SetForegroundWindow, SetWindowPos, ShowWindow, GA_ROOT, HWND_TOPMOST, SW_RESTORE,
        SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
    };

    const MIN_CLIENT_W: i32 = 120;
    const MIN_CLIENT_H: i32 = 80;
    const OCR_MAX_W: u32 = 1280;

    #[derive(Clone)]
    struct FocusHit {
        hwnd: u64,
        title: String,
        exe: String,
    }

    static LAST_FOCUS: Mutex<Option<FocusHit>> = Mutex::new(None);

    struct EnumData {
        out: Vec<WindowInfo>,
        areas: Vec<i64>,
        needle: Option<String>,
        cursor_hwnds: Vec<HWND>,
    }

    pub fn pop_cursor() -> Result<String, String> {
        let mut data = EnumData {
            out: Vec::new(),
            areas: Vec::new(),
            needle: None,
            cursor_hwnds: Vec::new(),
        };
        unsafe {
            EnumWindows(Some(enum_proc), LPARAM(&mut data as *mut _ as isize))
                .map_err(|e| e.to_string())?;
        }
        let hwnd = data
            .cursor_hwnds
            .into_iter()
            .next()
            .ok_or("no Cursor window")?;
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(hwnd);
        }
        Ok("popped Cursor".into())
    }

    pub fn list_windows() -> Result<Vec<WindowInfo>, String> {
        let mut data = EnumData {
            out: Vec::new(),
            areas: Vec::new(),
            needle: None,
            cursor_hwnds: Vec::new(),
        };
        unsafe {
            EnumWindows(Some(enum_proc), LPARAM(&mut data as *mut _ as isize))
                .map_err(|e| e.to_string())?;
        }
        Ok(data.out)
    }

    pub fn start_focus_watch() {
        std::thread::Builder::new()
            .name("lingua-focus".into())
            .spawn(|| loop {
                note_foreground();
                std::thread::sleep(Duration::from_millis(250));
            })
            .ok();
    }

    pub fn capture_and_ocr(title_substr: &str) -> Result<OcrOut, String> {
        let hwnd = resolve_hwnd(title_substr.trim())?;
        let title = window_title(hwnd);
        let exe = exe_path(hwnd);
        if unsafe { IsIconic(hwnd).as_bool() } {
            // Never ShowWindow the target. WinUI WhatsApp treats restore/hide as close.
            return Err(if !may_show_window_on_target(&title, &exe) {
                "WhatsApp is minimized — restore the chat, then OCR".into()
            } else {
                "chat window is minimized — restore it, then OCR".into()
            });
        }
        let win = window_bounds(hwnd)?;
        let parked = park_overlapping_own(win);
        if !parked.is_empty() {
            std::thread::sleep(Duration::from_millis(50));
        }
        let grabbed = grab_bgra(hwnd);
        restore_parked(&parked);
        let (mut bgra, mut w, mut h) = grabbed?;
        if looks_like_whatsapp(&title, &exe) {
            let (x, y, cw, ch) = chat_pane_crop(w, h);
            let cropped = crop_bgra(&bgra, w, h, x, y, cw, ch);
            if !is_blank_bgra(&cropped.0) {
                (bgra, w, h) = cropped;
            }
        }
        let (bgra, w, h) = scale_bgra(bgra, w, h, OCR_MAX_W);
        let (plain, spans) = ocr_spans(bgra, w, h)?;
        let text = require_ocr_text(crate::ocr_text::format_ocr(&plain, &spans))?;
        Ok(OcrOut {
            text,
            source: title,
        })
    }

    fn window_title(hwnd: HWND) -> String {
        let mut buf = [0u16; 512];
        let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
        if n <= 0 {
            String::new()
        } else {
            String::from_utf16_lossy(&buf[..n as usize])
        }
    }

    fn exe_path(hwnd: HWND) -> String {
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pid == 0 {
            return String::new();
        }
        let proc = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) };
        let Ok(proc) = proc else {
            return String::new();
        };
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let ok = unsafe {
            QueryFullProcessImageNameW(proc, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)
        };
        let _ = unsafe { CloseHandle(proc) };
        if ok.is_err() {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..len as usize])
    }

    fn is_pulse_target(title: &str, exe: &str) -> bool {
        let t = title.to_ascii_lowercase();
        let e = exe.to_ascii_lowercase();
        t.contains("pulse")
            || e.contains("pulse.exe")
            || t.contains("lingua")
            || e.contains("lingua.exe")
    }

    fn hwnd_alive(hwnd: HWND) -> bool {
        unsafe {
            IsWindow(Some(hwnd)).as_bool() && IsWindowVisible(hwnd).as_bool() && !is_cloaked(hwnd)
        }
    }

    fn note_foreground() {
        let raw = unsafe { GetForegroundWindow() };
        if raw.0.is_null() {
            return;
        }
        let hwnd = unsafe { GetAncestor(raw, GA_ROOT) };
        let hwnd = if hwnd.0.is_null() { raw } else { hwnd };
        let title = window_title(hwnd);
        let exe = exe_path(hwnd);
        if title.is_empty() || is_pulse_target(&title, &exe) {
            return;
        }
        let (_, _, area) = client_area(hwnd);
        if area < i64::from(MIN_CLIENT_W) * i64::from(MIN_CLIENT_H) {
            return;
        }
        if let Ok(mut guard) = LAST_FOCUS.lock() {
            *guard = Some(FocusHit {
                hwnd: hwnd.0 as usize as u64,
                title,
                exe,
            });
        }
    }

    fn last_focus() -> Option<FocusHit> {
        LAST_FOCUS.lock().ok()?.clone()
    }

    fn resolve_hwnd(needle: &str) -> Result<HWND, String> {
        if let Some(hit) = last_focus() {
            let hwnd = HWND(hit.hwnd as usize as *mut core::ffi::c_void);
            if hwnd_alive(hwnd) {
                let n = needle.to_ascii_lowercase();
                if looks_like_whatsapp(&hit.title, &hit.exe)
                    || (!n.is_empty() && hit.title.to_ascii_lowercase().contains(&n))
                {
                    return Ok(hwnd);
                }
            }
        }
        if !needle.is_empty() {
            if let Some(hwnd) = hwnd_matching(needle) {
                return Ok(hwnd);
            }
        }
        if looks_like_whatsapp(needle, "") {
            if let Some(hwnd) = hwnd_matching("whatsapp") {
                return Ok(hwnd);
            }
        }
        Err(if needle.is_empty() {
            "no focused chat — click the WhatsApp conversation first".into()
        } else {
            format!("no window matching '{needle}'")
        })
    }

    fn hwnd_matching(needle: &str) -> Option<HWND> {
        let mut data = EnumData {
            out: Vec::new(),
            areas: Vec::new(),
            needle: Some(needle.to_ascii_lowercase()),
            cursor_hwnds: Vec::new(),
        };
        unsafe {
            EnumWindows(Some(enum_proc), LPARAM(&mut data as *mut _ as isize)).ok()?;
        }
        let ranked: Vec<(u64, i64)> = data
            .out
            .iter()
            .zip(data.areas.iter())
            .map(|(w, a)| (w.hwnd, *a))
            .collect();
        let raw = pick_largest_hwnd(&ranked)?;
        Some(HWND(raw as usize as *mut core::ffi::c_void))
    }

    fn is_cloaked(hwnd: HWND) -> bool {
        let mut cloaked = 0u32;
        let ok = unsafe {
            DwmGetWindowAttribute(
                hwnd,
                DWMWA_CLOAKED,
                &mut cloaked as *mut u32 as *mut core::ffi::c_void,
                size_of::<u32>() as u32,
            )
        };
        ok.is_ok() && cloaked != 0
    }

    fn client_area(hwnd: HWND) -> (i32, i32, i64) {
        let mut rc = RECT::default();
        if unsafe { GetClientRect(hwnd, &mut rc) }.is_err() {
            return (0, 0, 0);
        }
        let w = (rc.right - rc.left).max(0);
        let h = (rc.bottom - rc.top).max(0);
        (w, h, i64::from(w) * i64::from(h))
    }

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let data = &mut *(lparam.0 as *mut EnumData);
        if !IsWindowVisible(hwnd).as_bool() || is_cloaked(hwnd) {
            return BOOL(1);
        }
        let mut buf = [0u16; 512];
        let n = GetWindowTextW(hwnd, &mut buf);
        if n <= 0 {
            return BOOL(1);
        }
        let title = String::from_utf16_lossy(&buf[..n as usize]);
        if title.is_empty() {
            return BOOL(1);
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let lower = title.to_ascii_lowercase();
        if lower.contains("cursor") && !lower.contains("pulse") {
            data.cursor_hwnds.push(hwnd);
        }
        let (cw, ch, area) = client_area(hwnd);
        if let Some(needle) = &data.needle {
            let exe = exe_path(hwnd);
            let hit = lower.contains(needle)
                || (needle.contains("whatsapp") && looks_like_whatsapp(&title, &exe));
            if hit && cw >= MIN_CLIENT_W && ch >= MIN_CLIENT_H {
                data.out.push(WindowInfo {
                    title: title.clone(),
                    hwnd: hwnd.0 as usize as u64,
                });
                data.areas.push(area);
            }
        } else if title.len() > 1 && cw >= MIN_CLIENT_W && ch >= MIN_CLIENT_H {
            data.out.push(WindowInfo {
                title,
                hwnd: hwnd.0 as usize as u64,
            });
        }
        BOOL(1)
    }

    fn window_bounds(hwnd: HWND) -> Result<RECT, String> {
        let mut rc = RECT::default();
        let ok = unsafe {
            DwmGetWindowAttribute(
                hwnd,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                &mut rc as *mut RECT as *mut core::ffi::c_void,
                size_of::<RECT>() as u32,
            )
        };
        if ok.is_ok() && rc.right > rc.left && rc.bottom > rc.top {
            return Ok(rc);
        }
        unsafe { GetWindowRect(hwnd, &mut rc) }.map_err(|e| e.to_string())?;
        Ok(rc)
    }

    fn rect_contains(outer: RECT, x: i32, y: i32) -> bool {
        x >= outer.left && x < outer.right && y >= outer.top && y < outer.bottom
    }

    fn find_output(window: RECT) -> Result<(IDXGIAdapter, IDXGIOutput1, RECT), String> {
        let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.map_err(|e| e.to_string())?;
        let cx = window.left + (window.right - window.left) / 2;
        let cy = window.top + (window.bottom - window.top) / 2;
        let mut i = 0;
        loop {
            let adapter = match unsafe { factory.EnumAdapters(i) } {
                Ok(a) => a,
                Err(_) => break,
            };
            i += 1;
            let mut j = 0;
            loop {
                let output = match unsafe { adapter.EnumOutputs(j) } {
                    Ok(o) => o,
                    Err(_) => break,
                };
                j += 1;
                let desc = match unsafe { output.GetDesc() } {
                    Ok(d) => d,
                    Err(_) => continue,
                };
                if !desc.AttachedToDesktop.as_bool() {
                    continue;
                }
                if rect_contains(desc.DesktopCoordinates, cx, cy) {
                    let output1: IDXGIOutput1 = output.cast().map_err(|e| e.to_string())?;
                    return Ok((adapter, output1, desc.DesktopCoordinates));
                }
            }
        }
        Err("no monitor contains that window".into())
    }

    struct OwnWindows {
        pid: u32,
        hwnds: Vec<HWND>,
    }

    unsafe extern "system" fn own_visible_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let data = &mut *(lparam.0 as *mut OwnWindows);
        if !IsWindowVisible(hwnd).as_bool() {
            return BOOL(1);
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == data.pid {
            data.hwnds.push(hwnd);
        }
        BOOL(1)
    }

    fn rects_overlap(a: RECT, b: RECT) -> bool {
        a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top
    }

    struct Parked {
        hwnd: HWND,
        left: i32,
        top: i32,
    }

    fn park_overlapping_own(win: RECT) -> Vec<Parked> {
        let pulse_pid = unsafe { GetCurrentProcessId() };
        let mut data = OwnWindows {
            pid: pulse_pid,
            hwnds: Vec::new(),
        };
        unsafe {
            let _ = EnumWindows(
                Some(own_visible_proc),
                LPARAM(&mut data as *mut _ as isize),
            );
        }
        let mut parked = Vec::new();
        for hwnd in data.hwnds {
            let Ok(rc) = window_bounds(hwnd) else { continue };
            let title = window_title(hwnd);
            let exe = exe_path(hwnd);
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
            if !should_park_overlay(pid, pulse_pid, &title, &exe, rects_overlap(rc, win)) {
                continue;
            }
            let (x, y) = park_position(rc.left, rc.top, (rc.right - rc.left).max(1));
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    x,
                    y,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER,
                );
            }
            parked.push(Parked {
                hwnd,
                left: rc.left,
                top: rc.top,
            });
        }
        parked
    }

    fn restore_parked(parked: &[Parked]) {
        for p in parked {
            unsafe {
                let _ = SetWindowPos(
                    p.hwnd,
                    Some(HWND_TOPMOST),
                    p.left,
                    p.top,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER,
                );
            }
        }
    }

    fn grab_bgra(hwnd: HWND) -> Result<(Vec<u8>, u32, u32), String> {
        let win = window_bounds(hwnd)?;
        let w = win.right - win.left;
        let h = win.bottom - win.top;
        if w < MIN_CLIENT_W || h < MIN_CLIENT_H {
            return Err("window too small to capture".into());
        }
        let (adapter, output, desktop) = find_output(win)?;
        let mut device = None;
        unsafe {
            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                None,
            )
        }
        .map_err(|e| e.to_string())?;
        let device = device.ok_or("D3D11CreateDevice returned no device")?;
        let context = unsafe { device.GetImmediateContext() }.map_err(|e| e.to_string())?;
        let dup = unsafe { output.DuplicateOutput(&device) }
            .map_err(|e| format!("screen capture blocked ({e})"))?;

        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource: Option<IDXGIResource> = None;
        let mut last = "screen capture timed out".to_string();
        for _ in 0..8 {
            match unsafe { dup.AcquireNextFrame(200, &mut info, &mut resource) } {
                Ok(()) => {
                    let tex: ID3D11Texture2D = match resource.take() {
                        Some(r) => r.cast().map_err(|e| e.to_string())?,
                        None => {
                            let _ = unsafe { dup.ReleaseFrame() };
                            last = "screen capture returned no frame".into();
                            continue;
                        }
                    };
                    let copied = copy_crop(&device, &context, &tex, win, desktop);
                    let _ = unsafe { dup.ReleaseFrame() };
                    match copied {
                        Ok(img) if !is_blank_bgra(&img.0) => return Ok(img),
                        Ok(_) => last = "capture was blank — is WhatsApp uncovered?".into(),
                        Err(e) => last = e,
                    }
                }
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => continue,
                Err(e) => return Err(format!("screen capture failed ({e})")),
            }
        }
        Err(last)
    }

    fn copy_crop(
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        src: &ID3D11Texture2D,
        win: RECT,
        desktop: RECT,
    ) -> Result<(Vec<u8>, u32, u32), String> {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { src.GetDesc(&mut desc) };
        // 87 = B8G8R8A8_UNORM, 91 = B8G8R8A8_UNORM_SRGB
        if desc.Format.0 != DXGI_FORMAT_B8G8R8A8_UNORM.0 && desc.Format.0 != 91 {
            return Err(format!("unexpected desktop pixel format {}", desc.Format.0));
        }
        desc.Usage = D3D11_USAGE_STAGING;
        desc.BindFlags = 0;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        desc.MiscFlags = 0;
        let mut staging = None;
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut staging)) }
            .map_err(|e| e.to_string())?;
        let staging = staging.ok_or("staging texture missing")?;
        unsafe { context.CopyResource(&staging, src) };
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .map_err(|e| e.to_string())?;
        }
        let tex_w = desc.Width as i32;
        let tex_h = desc.Height as i32;
        let x0 = (win.left - desktop.left).clamp(0, tex_w.saturating_sub(1));
        let y0 = (win.top - desktop.top).clamp(0, tex_h.saturating_sub(1));
        let x1 = (win.right - desktop.left).clamp(x0 + 1, tex_w);
        let y1 = (win.bottom - desktop.top).clamp(y0 + 1, tex_h);
        let w = (x1 - x0) as u32;
        let h = (y1 - y0) as u32;
        let pitch = mapped.RowPitch as usize;
        let src_ptr = mapped.pData as *const u8;
        let mut out = vec![0u8; w as usize * h as usize * 4];
        for y in 0..h as usize {
            let src_row = unsafe { src_ptr.add((y0 as usize + y) * pitch + x0 as usize * 4) };
            let dst = &mut out[y * w as usize * 4..][..w as usize * 4];
            unsafe {
                std::ptr::copy_nonoverlapping(src_row, dst.as_mut_ptr(), w as usize * 4);
            }
            for px in dst.chunks_exact_mut(4) {
                px[3] = 255;
            }
        }
        unsafe { context.Unmap(&staging, 0) };
        Ok((out, w, h))
    }

    fn ocr_engine() -> Result<OcrEngine, String> {
        for tag in ["es", "en"] {
            if let Ok(lang) = Language::CreateLanguage(&tag.into()) {
                if let Ok(engine) = OcrEngine::TryCreateFromLanguage(&lang) {
                    return Ok(engine);
                }
            }
        }
        OcrEngine::TryCreateFromUserProfileLanguages().map_err(|e| e.to_string())
    }

    fn ocr_spans(
        bgra: Vec<u8>,
        w: u32,
        h: u32,
    ) -> Result<(String, Vec<crate::ocr_text::OcrSpan>), String> {
        let bmp_bytes = bgra_to_bmp32(&bgra, w, h);
        let stream = InMemoryRandomAccessStream::new().map_err(|e| e.to_string())?;
        let writer = DataWriter::CreateDataWriter(&stream).map_err(|e| e.to_string())?;
        writer.WriteBytes(&bmp_bytes).map_err(|e| e.to_string())?;
        writer
            .StoreAsync()
            .map_err(|e| e.to_string())?
            .get()
            .map_err(|e| e.to_string())?;
        writer.FlushAsync().map_err(|e| e.to_string())?.get().ok();
        stream.Seek(0).map_err(|e| e.to_string())?;
        let decoder = BitmapDecoder::CreateAsync(&stream)
            .map_err(|e| e.to_string())?
            .get()
            .map_err(|e| e.to_string())?;
        let software = decoder
            .GetSoftwareBitmapAsync()
            .map_err(|e| e.to_string())?
            .get()
            .map_err(|e| e.to_string())?;
        let engine = ocr_engine()?;
        let result = engine
            .RecognizeAsync(&software)
            .map_err(|e| e.to_string())?
            .get()
            .map_err(|e| e.to_string())?;
        let plain = result.Text().map_err(|e| e.to_string())?.to_string();
        let mut spans = Vec::new();
        if let Ok(lines) = result.Lines() {
            for line in lines {
                let Ok(text) = line.Text() else { continue };
                let text = text.to_string();
                if text.trim().is_empty() {
                    continue;
                }
                let (cx, cy) = line_center(&line, w, h);
                spans.push(crate::ocr_text::OcrSpan { text, cx, cy });
            }
        }
        Ok((plain, spans))
    }

    fn line_center(line: &windows::Media::Ocr::OcrLine, w: u32, h: u32) -> (f32, f32) {
        let Ok(words) = line.Words() else {
            return (0.5, 0.5);
        };
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = 0.0f32;
        let mut max_y = 0.0f32;
        let mut any = false;
        for word in words {
            let Ok(r) = word.BoundingRect() else { continue };
            any = true;
            min_x = min_x.min(r.X);
            min_y = min_y.min(r.Y);
            max_x = max_x.max(r.X + r.Width);
            max_y = max_y.max(r.Y + r.Height);
        }
        if !any || w == 0 || h == 0 {
            return (0.5, 0.5);
        }
        (
            ((min_x + max_x) * 0.5 / w as f32).clamp(0.0, 1.0),
            ((min_y + max_y) * 0.5 / h as f32).clamp(0.0, 1.0),
        )
    }

    fn bgra_to_bmp32(bgra: &[u8], w: u32, h: u32) -> Vec<u8> {
        let row = w * 4;
        let pixel_size = (row * h) as usize;
        let file_size = 54 + pixel_size;
        let mut out = Vec::with_capacity(file_size);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&(file_size as u32).to_le_bytes());
        out.extend_from_slice(&[0u8; 4]);
        out.extend_from_slice(&54u32.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&w.to_le_bytes());
        out.extend_from_slice(&(-(h as i32)).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(pixel_size as u32).to_le_bytes());
        out.extend_from_slice(&[0u8; 16]);
        out.extend_from_slice(bgra);
        out
    }
}

#[cfg(windows)]
pub use imp::{capture_and_ocr, list_windows, pop_cursor, start_focus_watch};

#[cfg(test)]
mod tests {
    use super::{
        chat_pane_crop, crop_bgra, is_blank_bgra, looks_like_whatsapp, mask_bgra,
        may_show_window_on_target, park_position, pick_largest_hwnd, require_ocr_text, scale_bgra,
        should_park_overlay,
    };

    fn solid(w: u32, h: u32, b: u8, g: u8, r: u8) -> Vec<u8> {
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..(w * h) {
            out.extend_from_slice(&[b, g, r, 255]);
        }
        out
    }

    fn two_tone(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                if x < w / 2 {
                    out.extend_from_slice(&[15, 20, 26, 255]);
                } else if y % 3 == 0 {
                    out.extend_from_slice(&[230, 235, 240, 255]);
                } else {
                    out.extend_from_slice(&[15, 20, 26, 255]);
                }
            }
        }
        out
    }

    #[test]
    fn blank_when_all_black_or_solid() {
        assert!(is_blank_bgra(&[]));
        assert!(is_blank_bgra(&solid(64, 64, 0, 0, 0)));
        assert!(is_blank_bgra(&solid(64, 64, 18, 18, 18)));
    }

    #[test]
    fn not_blank_when_dark_theme_has_text_contrast() {
        assert!(!is_blank_bgra(&two_tone(64, 64)));
    }

    #[test]
    fn empty_ocr_is_an_error() {
        assert!(require_ocr_text(String::new()).is_err());
        assert!(require_ocr_text("   \n".into()).is_err());
        assert_eq!(require_ocr_text("Hola".into()).unwrap(), "Hola");
    }

    #[test]
    fn picks_largest_visible_match() {
        assert_eq!(pick_largest_hwnd(&[]), None);
        assert_eq!(
            pick_largest_hwnd(&[(1, 0), (2, 800 * 600), (3, 200 * 100)]),
            Some(2)
        );
    }

    #[test]
    fn scales_wide_bitmap_down() {
        let (out, w, h) = scale_bgra(solid(8, 4, 10, 20, 30), 8, 4, 4);
        assert_eq!((w, h), (4, 2));
        assert_eq!(out.len(), 4 * 2 * 4);
        assert_eq!(&out[0..4], &[10, 20, 30, 255]);
    }

    #[test]
    fn whatsapp_from_title_or_exe() {
        assert!(looks_like_whatsapp("WhatsApp", ""));
        assert!(looks_like_whatsapp("Mom", r"C:\Program Files\WindowsApps\WhatsApp.exe"));
        assert!(!looks_like_whatsapp("Cursor", r"C:\Users\x\Cursor.exe"));
    }

    #[test]
    fn chat_pane_drops_sidebar_on_wide_window() {
        let (x, y, cw, ch) = chat_pane_crop(1200, 800);
        assert!(x >= 300);
        assert!(y >= 40);
        assert!(cw + x == 1200);
        assert!(ch < 800);
        assert!(ch >= 80);
    }

    #[test]
    fn crop_bgra_keeps_requested_block() {
        let mut src = vec![0u8; 4 * 3 * 4];
        src[2 * 4 + 0] = 9;
        let (out, w, h) = crop_bgra(&src, 4, 3, 2, 0, 2, 1);
        assert_eq!((w, h), (2, 1));
        assert_eq!(out[0], 9);
    }

    #[test]
    fn mask_bgra_blacks_out_the_overlay() {
        let mut src = vec![9u8; 4 * 2 * 4];
        mask_bgra(&mut src, 4, 2, 1, 0, 2, 1);
        assert_eq!(&src[4..8], &[0, 0, 0, 255]);
        assert_eq!(src[0], 9);
        assert_eq!(src[12], 9);
    }

    #[test]
    fn never_parks_whatsapp_even_if_pid_matches_and_overlaps() {
        assert!(!should_park_overlay(
            42,
            42,
            "Juan — WhatsApp",
            r"C:\Program Files\WindowsApps\WhatsApp.exe",
            true,
        ));
        assert!(!should_park_overlay(
            42,
            42,
            "Mom",
            r"C:\Program Files\WindowsApps\WhatsApp.exe",
            true,
        ));
    }

    #[test]
    fn parks_only_pulse_overlay_that_overlaps() {
        let exe = r"C:\Users\x\AppData\Local\pulse\pulse.exe";
        assert!(should_park_overlay(7, 7, "pulse", exe, true));
        assert!(!should_park_overlay(7, 7, "pulse", exe, false));
        assert!(!should_park_overlay(7, 99, "pulse", exe, true));
    }

    #[test]
    fn never_parks_other_apps_even_when_they_overlap() {
        assert!(!should_park_overlay(
            1,
            1,
            "Gmail",
            r"C:\Program Files\Google\Chrome\chrome.exe",
            true,
        ));
        assert!(!should_park_overlay(1, 1, "Cursor", r"C:\Users\x\Cursor.exe", true));
    }

    #[test]
    fn never_showwindow_whatsapp_hwnd() {
        assert!(!may_show_window_on_target(
            "WhatsApp",
            r"C:\Program Files\WindowsApps\WhatsApp.exe"
        ));
        assert!(!may_show_window_on_target(
            "Mom",
            r"C:\Program Files\WindowsApps\WhatsApp.exe"
        ));
        assert!(may_show_window_on_target(
            "pulse",
            r"C:\Users\x\AppData\Local\pulse\pulse.exe"
        ));
    }

    #[test]
    fn park_moves_overlay_off_every_monitor_without_changing_y() {
        assert_eq!(park_position(100, 40, 960), (-32000, 40));
        assert!(park_position(0, 0, 1920).0 < -10000);
    }
}
