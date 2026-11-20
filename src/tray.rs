use crate::config::{HotkeySpec, ShortcutConfig};
use std::mem::size_of;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMNCRP_DISABLED, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_NCRENDERING_POLICY,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
    DeleteObject,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegDeleteValueW,
    RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, RegisterHotKey,
    ReleaseCapture, UnregisterHotKey,
};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyIcon, DestroyMenu, DestroyWindow, DispatchMessageW, GWL_EXSTYLE, GWL_STYLE,
    GetCursorPos, GetMessageW, GetWindowLongPtrW, GetWindowRect, HICON, HTCAPTION, HWND_NOTOPMOST,
    HWND_TOPMOST, ICONINFO, MF_SEPARATOR, MF_STRING, MSG, PostMessageW, PostQuitMessage,
    RegisterClassW, SW_HIDE, SW_SHOW, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_SHOWWINDOW, SendMessageW, SetForegroundWindow,
    SetWindowDisplayAffinity, SetWindowLongPtrW, SetWindowPos, ShowWindow, TPM_BOTTOMALIGN,
    TPM_LEFTALIGN, TrackPopupMenu, TranslateMessage, WDA_EXCLUDEFROMCAPTURE, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_APP, WM_COMMAND, WM_DESTROY, WM_HOTKEY, WM_LBUTTONDBLCLK, WM_LBUTTONUP,
    WM_NCLBUTTONDOWN, WM_RBUTTONUP, WNDCLASSW, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP, WS_VISIBLE,
};
use windows::core::PCWSTR;

const WM_TRAY_CALLBACK: u32 = WM_APP + 1;
const WM_UPDATE_HOTKEYS: u32 = WM_APP + 2;
const WM_SHUTDOWN_TRAY: u32 = WM_APP + 3;

const HK_REGION: i32 = 1;
const HK_FULLSCREEN: i32 = 2;
const HK_TOGGLE_HUD: i32 = 3;

const MENU_SHOW: usize = 1001;
const MENU_REGION: usize = 1002;
const MENU_FULL: usize = 1003;
const MENU_QUIT: usize = 1004;

#[derive(Debug, Clone, Copy)]
pub enum TrayCommand {
    ShowOrToggleHud,
    ShowWindow,
    CaptureFull,
    CaptureRegion,
    Quit,
}

struct SharedTrayState {
    sender: async_channel::Sender<TrayCommand>,
    shortcuts: Mutex<ShortcutConfig>,
}

static TRAY_STATE: OnceLock<Arc<SharedTrayState>> = OnceLock::new();

#[derive(Clone)]
pub struct TrayController {
    tray_hwnd: isize,
    state: Arc<SharedTrayState>,
}

impl TrayController {
    /// Spawns a dedicated Win32 message thread for the System Tray icon + Global Hotkeys.
    /// Blocks in kernel `GetMessageW` -> 0.00% CPU usage while idle.
    pub fn spawn(
        initial_shortcuts: ShortcutConfig,
    ) -> (Self, async_channel::Receiver<TrayCommand>) {
        let (tx, rx) = async_channel::unbounded();
        let state = Arc::new(SharedTrayState {
            sender: tx,
            shortcuts: Mutex::new(initial_shortcuts),
        });
        let _ = TRAY_STATE.set(Arc::clone(&state));

        let (hwnd_tx, hwnd_rx) = std::sync::mpsc::channel::<isize>();

        thread::Builder::new()
            .name("sniptone-tray-hotkeys".into())
            .spawn(move || unsafe {
                run_tray_loop(hwnd_tx);
            })
            .expect("failed to spawn tray thread");

        let tray_hwnd = hwnd_rx.recv().unwrap_or(0);
        (Self { tray_hwnd, state }, rx)
    }

    pub fn update_shortcuts(&self, shortcuts: ShortcutConfig) {
        if let Ok(mut guard) = self.state.shortcuts.lock() {
            *guard = shortcuts;
        }
        if self.tray_hwnd != 0 {
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(self.tray_hwnd as *mut _)),
                    WM_UPDATE_HOTKEYS,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        }
    }

    pub fn shutdown(&self) {
        if self.tray_hwnd != 0 {
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(self.tray_hwnd as *mut _)),
                    WM_SHUTDOWN_TRAY,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        }
    }
}

/// Strips all Win32 non-client window chrome & DWM rectangular frame/shadow so DirectComposition's
/// premultiplied-alpha swapchain renders ONLY the rounded GPUI capsule/card with zero outer box.
unsafe fn strip_nonclient_chrome(hwnd: HWND) {
    unsafe {
        let cur_style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let visible_bit = cur_style & WS_VISIBLE.0;
        let popup_style = (WS_POPUP | WS_CLIPSIBLINGS | WS_CLIPCHILDREN).0 | visible_bit;
        let _ = SetWindowLongPtrW(hwnd, GWL_STYLE, popup_style as isize);

        let nc_policy = DWMNCRP_DISABLED;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_POLICY,
            &nc_policy as *const _ as *const _,
            size_of_val(&nc_policy) as u32,
        );
        let border_none: u32 = DWMWA_COLOR_NONE;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &border_none as *const u32 as *const _,
            size_of::<u32>() as u32,
        );
        let corner_pref = DWMWCP_DONOTROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner_pref as *const _ as *const _,
            size_of_val(&corner_pref) as u32,
        );
    }
}

/// Configures a floating PopUp GPUI window (Game Bar, Widget, or Region Snip):
/// excludes it from WGC captures, strips all OS/DWM box borders, and optionally pins it topmost.
pub fn configure_floating_window(raw_hwnd: isize, topmost: bool) {
    if raw_hwnd != 0 {
        unsafe {
            let hwnd = HWND(raw_hwnd as *mut _);
            let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
            strip_nonclient_chrome(hwnd);
            let z = if topmost { HWND_TOPMOST } else { HWND_NOTOPMOST };
            let _ = SetWindowPos(
                hwnd,
                Some(z),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
    }
}

/// Configures an independent floating tooltip window:
/// click-through (`WS_EX_TRANSPARENT`), non-focus-stealing (`WS_EX_NOACTIVATE`), hidden from taskbar/Alt+Tab
/// (`WS_EX_TOOLWINDOW`), topmost, excluded from captures, and with zero OS/DWM frame chrome.
pub fn configure_tooltip_window(raw_hwnd: isize) {
    if raw_hwnd != 0 {
        unsafe {
            let hwnd = HWND(raw_hwnd as *mut _);
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let extra = (WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_TOPMOST).0
                as isize;
            let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | extra);
            let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
            strip_nonclient_chrome(hwnd);
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
    }
}

/// Moves and resizes a floating tooltip without ever activating or stealing focus.
pub fn move_tooltip_physical(raw_hwnd: isize, left: i32, top: i32, width: i32, height: i32) {
    if raw_hwnd != 0 {
        unsafe {
            let hwnd = HWND(raw_hwnd as *mut _);
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                left,
                top,
                width,
                height,
                SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_SHOWWINDOW,
            );
        }
    }
}

pub fn set_window_topmost(raw_hwnd: isize, topmost: bool) {
    if raw_hwnd != 0 {
        unsafe {
            let hwnd = HWND(raw_hwnd as *mut _);
            let z = if topmost { HWND_TOPMOST } else { HWND_NOTOPMOST };
            let _ = SetWindowPos(
                hwnd,
                Some(z),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }
}

/// Starts a native Win32 window drag so any floating bar or widget glides smoothly at monitor
/// refresh rate across horizontal and vertical monitors with differing DPIs.
pub fn begin_native_window_drag(raw_hwnd: isize) {
    if raw_hwnd != 0 {
        unsafe {
            let hwnd = HWND(raw_hwnd as *mut _);
            let _ = ReleaseCapture();
            let _ = SendMessageW(
                hwnd,
                WM_NCLBUTTONDOWN,
                Some(WPARAM(HTCAPTION as usize)),
                Some(LPARAM(0)),
            );
        }
    }
}

pub fn get_window_rect(raw_hwnd: isize) -> Option<(i32, i32, i32, i32)> {
    if raw_hwnd == 0 {
        return None;
    }
    unsafe {
        let mut rect = RECT::default();
        if GetWindowRect(HWND(raw_hwnd as *mut _), &mut rect).is_ok() {
            Some((
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
            ))
        } else {
            None
        }
    }
}

pub fn move_window_physical(
    raw_hwnd: isize,
    left: i32,
    top: i32,
    width: i32,
    height: i32,
    topmost: bool,
) {
    if raw_hwnd != 0 {
        unsafe {
            let hwnd = HWND(raw_hwnd as *mut _);
            let z = if topmost { HWND_TOPMOST } else { HWND_NOTOPMOST };
            // Two passes handle cross-monitor WM_DPICHANGED cleanly (e.g. 2560x1440 -> 1080x1920)
            for _ in 0..2 {
                let _ = SetWindowPos(
                    hwnd,
                    Some(z),
                    left,
                    top,
                    width,
                    height,
                    SWP_FRAMECHANGED | SWP_SHOWWINDOW,
                );
            }
        }
    }
}

pub fn hide_window(raw_hwnd: isize) {
    if raw_hwnd != 0 {
        unsafe {
            let _ = ShowWindow(HWND(raw_hwnd as *mut _), SW_HIDE);
        }
    }
}

pub fn show_window(raw_hwnd: isize, topmost: bool, focus: bool) {
    if raw_hwnd != 0 {
        unsafe {
            let hwnd = HWND(raw_hwnd as *mut _);
            let _ = ShowWindow(hwnd, SW_SHOW);
            let z = if topmost { HWND_TOPMOST } else { HWND_NOTOPMOST };
            let flags = if focus {
                SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW
            } else {
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW
            };
            let _ = SetWindowPos(hwnd, Some(z), 0, 0, 0, 0, flags);
            if focus {
                let _ = SetForegroundWindow(hwnd);
            }
        }
    }
}

fn to_wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

const RUN_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const STARTUP_VALUE_NAME: &str = "SnipTone";

/// Returns whether SnipTone is registered under `HKCU\...\Run` to launch at Windows sign-in.
pub fn is_windows_startup_enabled() -> bool {
    let subkey = to_wide_null(RUN_SUBKEY);
    let value = to_wide_null(STARTUP_VALUE_NAME);
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            Some(0),
            KEY_QUERY_VALUE,
            &mut hkey,
        )
        .is_err()
        {
            return false;
        }
        let exists = RegQueryValueExW(
            hkey,
            PCWSTR(value.as_ptr()),
            None,
            None,
            None,
            None,
        )
        .is_ok();
        let _ = RegCloseKey(hkey);
        exists
    }
}

/// Registers or removes SnipTone in `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
/// (launches quietly in System Tray via `--tray` with 0% idle CPU; requires no admin rights).
pub fn set_windows_startup_enabled(enabled: bool) -> Result<(), String> {
    let subkey = to_wide_null(RUN_SUBKEY);
    let value = to_wide_null(STARTUP_VALUE_NAME);
    unsafe {
        let mut hkey = HKEY::default();
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            Some(0),
            KEY_SET_VALUE,
            &mut hkey,
        )
        .ok()
        .map_err(|e| format!("Registry open failed: {e}"))?;

        let result = if enabled {
            let exe = std::env::current_exe()
                .map_err(|e| format!("Resolve executable path failed: {e}"))?;
            let cmd = format!("\"{}\" --tray", exe.display());
            let wide = to_wide_null(&cmd);
            let bytes = std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2);
            RegSetValueExW(
                hkey,
                PCWSTR(value.as_ptr()),
                Some(0),
                REG_SZ,
                Some(bytes),
            )
            .ok()
            .map_err(|e| format!("Registry write failed: {e}"))
        } else {
            let _ = RegDeleteValueW(hkey, PCWSTR(value.as_ptr()));
            Ok(())
        };

        let _ = RegCloseKey(hkey);
        result
    }
}

fn modifiers_for(spec: HotkeySpec) -> HOT_KEY_MODIFIERS {
    let mut mods = MOD_NOREPEAT;
    if spec.ctrl {
        mods |= MOD_CONTROL;
    }
    if spec.shift {
        mods |= MOD_SHIFT;
    }
    if spec.alt {
        mods |= MOD_ALT;
    }
    mods
}

unsafe fn register_all_hotkeys(hwnd: HWND) {
    let Some(state) = TRAY_STATE.get() else {
        return;
    };
    let shortcuts = state.shortcuts.lock().map(|g| g.clone()).unwrap_or_default();

    unsafe {
        let _ = UnregisterHotKey(Some(hwnd), HK_REGION);
        let _ = UnregisterHotKey(Some(hwnd), HK_FULLSCREEN);
        let _ = UnregisterHotKey(Some(hwnd), HK_TOGGLE_HUD);

        let _ = RegisterHotKey(
            Some(hwnd),
            HK_REGION,
            modifiers_for(shortcuts.region_snip),
            shortcuts.region_snip.vk,
        );
        let _ = RegisterHotKey(
            Some(hwnd),
            HK_FULLSCREEN,
            modifiers_for(shortcuts.full_screen),
            shortcuts.full_screen.vk,
        );
        let _ = RegisterHotKey(
            Some(hwnd),
            HK_TOGGLE_HUD,
            modifiers_for(shortcuts.toggle_hud),
            shortcuts.toggle_hud.vk,
        );
    }
}

/// Embedded 32x32 BGRA pixels of the official SnipTone 01A (HDR Color Spectrum Fill) identity icon.
const TRAY_ICON_BGRA: &[u8] = include_bytes!("../assets/sniptone_tray_32.bgra");

/// Builds the crisp 32x32 RGBA System Tray icon from the embedded SnipTone 01A identity asset.
unsafe fn create_tray_icon() -> Option<HICON> {
    const SIZE: i32 = 32;
    const EXPECTED_BYTES: usize = (SIZE * SIZE * 4) as usize;
    if TRAY_ICON_BGRA.len() != EXPECTED_BYTES {
        return None;
    }

    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: SIZE,
            biHeight: -SIZE,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    unsafe {
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let color_bmp = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        if bits.is_null() {
            let _ = DeleteObject(color_bmp.into());
            return None;
        }

        let pixels = std::slice::from_raw_parts_mut(bits as *mut u8, EXPECTED_BYTES);
        pixels.copy_from_slice(TRAY_ICON_BGRA);

        let mask_bmp = CreateBitmap(SIZE, SIZE, 1, 1, None);
        let info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask_bmp,
            hbmColor: color_bmp,
        };
        let icon = CreateIconIndirect(&info).ok();
        let _ = DeleteObject(color_bmp.into());
        let _ = DeleteObject(mask_bmp.into());
        icon
    }
}

unsafe fn show_tray_context_menu(hwnd: HWND) {
    let shortcuts = TRAY_STATE
        .get()
        .and_then(|s| s.shortcuts.lock().ok().map(|g| g.clone()))
        .unwrap_or_default();

    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };

        let show_lbl = to_wide_null(&format!(
            "Show SnipTone\t{}",
            shortcuts.toggle_hud.display_label()
        ));
        let region_lbl = to_wide_null(&format!(
            "Region Snip\t{}",
            shortcuts.region_snip.display_label()
        ));
        let full_lbl = to_wide_null(&format!(
            "Full Screen\t{}",
            shortcuts.full_screen.display_label()
        ));
        let quit_lbl = to_wide_null("Quit SnipTone");

        let _ = AppendMenuW(menu, MF_STRING, MENU_SHOW, PCWSTR(show_lbl.as_ptr()));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, MENU_REGION, PCWSTR(region_lbl.as_ptr()));
        let _ = AppendMenuW(menu, MF_STRING, MENU_FULL, PCWSTR(full_lbl.as_ptr()));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, MENU_QUIT, PCWSTR(quit_lbl.as_ptr()));

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(menu, TPM_LEFTALIGN | TPM_BOTTOMALIGN, pt.x, pt.y, None, hwnd, None);
        let _ = DestroyMenu(menu);
    }
}

fn emit(cmd: TrayCommand) {
    if let Some(state) = TRAY_STATE.get() {
        let _ = state.sender.try_send(cmd);
    }
}

unsafe extern "system" fn tray_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_HOTKEY => {
            match wparam.0 as i32 {
                HK_REGION => emit(TrayCommand::CaptureRegion),
                HK_FULLSCREEN => emit(TrayCommand::CaptureFull),
                HK_TOGGLE_HUD => emit(TrayCommand::ShowOrToggleHud),
                _ => {}
            }
            LRESULT(0)
        }
        WM_TRAY_CALLBACK => {
            let ev = (lparam.0 & 0xFFFF) as u32;
            match ev {
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => emit(TrayCommand::ShowWindow),
                WM_RBUTTONUP => unsafe { show_tray_context_menu(hwnd) },
                _ => {}
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            match wparam.0 & 0xFFFF {
                MENU_SHOW => emit(TrayCommand::ShowWindow),
                MENU_REGION => emit(TrayCommand::CaptureRegion),
                MENU_FULL => emit(TrayCommand::CaptureFull),
                MENU_QUIT => emit(TrayCommand::Quit),
                _ => {}
            }
            LRESULT(0)
        }
        WM_UPDATE_HOTKEYS => {
            unsafe { register_all_hotkeys(hwnd) };
            LRESULT(0)
        }
        WM_SHUTDOWN_TRAY => {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

unsafe fn run_tray_loop(hwnd_tx: std::sync::mpsc::Sender<isize>) {
    unsafe {
        let class_name = to_wide_null("SnipToneTrayMessageWindow");
        let hinstance = GetModuleHandleW(None).unwrap_or_default();

        let wc = WNDCLASSW {
            lpfnWndProc: Some(tray_wnd_proc),
            hInstance: hinstance.into(),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassW(&wc);

        let hwnd = match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(class_name.as_ptr()),
            PCWSTR::null(),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinstance.into()),
            None,
        ) {
            Ok(h) => h,
            Err(_) => {
                let _ = hwnd_tx.send(0);
                return;
            }
        };

        let icon = create_tray_icon();
        let mut nid = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_TIP | if icon.is_some() { NIF_ICON } else { Default::default() },
            uCallbackMessage: WM_TRAY_CALLBACK,
            hIcon: icon.unwrap_or_default(),
            ..Default::default()
        };

        let tip = "SnipTone — HDR Capture & Tonemap";
        for (dst, src) in nid.szTip.iter_mut().zip(tip.encode_utf16()) {
            *dst = src;
        }
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);

        register_all_hotkeys(hwnd);
        let _ = hwnd_tx.send(hwnd.0 as isize);

        // Blocking kernel wait — 0.00% CPU usage in idle
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        let _ = UnregisterHotKey(Some(hwnd), HK_REGION);
        let _ = UnregisterHotKey(Some(hwnd), HK_FULLSCREEN);
        let _ = UnregisterHotKey(Some(hwnd), HK_TOGGLE_HUD);
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
        if let Some(hicon) = icon {
            let _ = DestroyIcon(hicon);
        }
    }
}
