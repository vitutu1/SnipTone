mod components;
mod gamebar;
pub mod icons;
mod snip;
mod store;
mod theme;
mod widgets;

use crate::config::AppConfig;
use crate::tray::{
    TrayCommand, TrayController, configure_floating_window, move_window_physical,
};
use gamebar::GameBarView;
use gpui::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use store::{
    AppStore, ManagedWindow, current_clock_string, extract_hwnd, gamebar_logical_size,
    hide_all_to_tray, place_gamebar, quit_application, show_all_from_tray, trigger_capture,
};

/// Boots the independent-window Game Bar HUD:
/// - Compact dockable Game Bar capsule (`666×58` horizontal or `58×594` vertical on Left/Right)
/// - All widget windows start off by default and open on demand as independent floating windows
/// - Zero-polling async bridge for System Tray & Global Hotkeys (0.00% idle CPU)
pub fn launch(
    config: AppConfig,
    tray: TrayController,
    tray_rx: async_channel::Receiver<TrayCommand>,
    start_hidden_in_tray: bool,
    cx: &mut App,
) {
    let hidden_in_tray = Arc::new(AtomicBool::new(start_hidden_in_tray));
    let allow_quit = Arc::new(AtomicBool::new(false));
    let bar_pos = config.bar_position;

    let store = cx.new(|_| {
        AppStore::new(
            config,
            tray,
            Arc::clone(&hidden_in_tray),
            Arc::clone(&allow_quit),
        )
    });

    // Open the compact floating Game Bar window anchored to the user's chosen edge
    let active_display = {
        let s = store.read(cx);
        s.displays.get(s.selected_display).cloned()
    };

    let (bar_w, bar_h) = gamebar_logical_size(bar_pos);
    let bar_bounds = Bounds::new(point(px(300.0), px(18.0)), size(px(bar_w), px(bar_h)));
    let store_bar = store.clone();
    let allow_quit_bar = Arc::clone(&allow_quit);

    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bar_bounds)),
            // `appears_transparent: false` disables GPUI's WM_NCCALCSIZE 8px frame inset & 4px HTTOP hijack,
            // while `WS_POPUP` + `DWMNCRP_DISABLED` + DirectComposition premultiplied alpha gives pure
            // borderless rounded rendering with zero outer DWM box!
            titlebar: Some(TitlebarOptions {
                appears_transparent: false,
                ..Default::default()
            }),
            kind: WindowKind::PopUp,
            window_background: WindowBackgroundAppearance::Opaque,
            window_decorations: Some(WindowDecorations::Client),
            is_movable: true,
            is_resizable: false,
            is_minimizable: false,
            focus: !start_hidden_in_tray,
            show: !start_hidden_in_tray,
            ..Default::default()
        },
        move |window, cx| {
            let hwnd = extract_hwnd(window);
            configure_floating_window(hwnd, true);

            if let Some(d) = &active_display {
                let scale = window.scale_factor();
                let bw = (bar_w * scale).round() as i32;
                let bh = (bar_h * scale).round() as i32;
                let work = d.live_work_area();
                let (bx, by) = place_gamebar(bar_pos, work, bw, bh, scale);
                move_window_physical(hwnd, bx, by, bw, bh, true);
            }

            let any_h = window.window_handle();
            store_bar.update(cx, |s, cx| {
                s.bar_window = Some(ManagedWindow {
                    handle: any_h,
                    hwnd,
                    pinned: true,
                });
                cx.notify();
            });

            // Intercept close on the Game Bar window -> hide all windows to System Tray
            let store_close = store_bar.clone();
            window.on_window_should_close(cx, move |_window, cx| {
                if allow_quit_bar.load(Ordering::SeqCst) {
                    true
                } else {
                    hide_all_to_tray(&store_close, cx);
                    false
                }
            });

            cx.new(|cx| GameBarView::new(store_bar, hwnd, window, cx))
        },
    )
    .expect("Failed to open SnipTone Game Bar");

    if start_hidden_in_tray {
        hide_all_to_tray(&store, cx);
    }

    // Zero-polling async listener for System Tray & Global Hotkey commands
    let store_tray = store.clone();
    cx.spawn(async move |cx| {
        while let Ok(cmd) = tray_rx.recv().await {
            let _ = cx.update(|cx| match cmd {
                TrayCommand::ShowWindow => {
                    store_tray.update(cx, |s, _| s.select_display_at_cursor());
                    show_all_from_tray(&store_tray, cx);
                }
                TrayCommand::ShowOrToggleHud => {
                    if store_tray.read(cx).hidden_in_tray.load(Ordering::SeqCst) {
                        store_tray.update(cx, |s, _| s.select_display_at_cursor());
                        show_all_from_tray(&store_tray, cx);
                    } else {
                        hide_all_to_tray(&store_tray, cx);
                    }
                }
                TrayCommand::CaptureFull => {
                    store_tray.update(cx, |s, _| s.select_display_at_cursor());
                    trigger_capture(&store_tray, 0, false, cx);
                }
                TrayCommand::CaptureRegion => {
                    store_tray.update(cx, |s, _| s.select_display_at_cursor());
                    trigger_capture(&store_tray, 0, true, cx);
                }
                TrayCommand::Quit => {
                    quit_application(&store_tray, cx);
                }
            });
        }
    })
    .detach();

    // Low-overhead 30s clock tick that does zero work while hidden in tray
    let store_clock = store.clone();
    let hidden_clock = Arc::clone(&hidden_in_tray);
    cx.spawn(async move |cx| loop {
        cx.background_executor().timer(Duration::from_secs(30)).await;
        if hidden_clock.load(Ordering::SeqCst) {
            continue;
        }
        let now = current_clock_string();
        let res = cx.update(|cx| {
            store_clock.update(cx, |s, cx| {
                if s.clock_str != now {
                    s.clock_str = now;
                    cx.notify();
                }
            });
        });
        if res.is_err() {
            break;
        }
    })
    .detach();

    cx.activate(true);
}
