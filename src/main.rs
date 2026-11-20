#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod capture;
mod clipboard;
mod config;
mod display;
mod export;
mod frame;
mod tonemap;
mod tray;
mod ui;

use config::AppConfig;
use gpui::*;
use std::time::Instant;
use tray::TrayController;
use ui::icons::IconsAssets;

fn run_probe() {
    println!("=== SnipTone Display & HDR Capture Probe ===");
    match display::enumerate_displays() {
        Ok(displays) => {
            if displays.is_empty() {
                println!("No active DXGI displays found.");
                return;
            }
            for d in &displays {
                println!(
                    "[{}] {} ({}) {}x{} @ ({},{}) work={}x{} @ ({},{}) rot={}° | HDR={} | SDR White={:.1} nits | Peak={:.1} nits | {} bpc | Primary={}",
                    d.id,
                    d.friendly_name,
                    d.gdi_name,
                    d.width,
                    d.height,
                    d.left,
                    d.top,
                    d.work_area.width,
                    d.work_area.height,
                    d.work_area.left,
                    d.work_area.top,
                    d.rotation.degrees(),
                    d.is_hdr,
                    d.sdr_white_nits,
                    d.max_luminance_nits,
                    d.bits_per_color,
                    d.is_primary,
                );
            }

            for target in &displays {
                println!("\nCapturing '[{}] {}'...", target.id, target.friendly_name);
                let t0 = Instant::now();
                match capture::capture_display(target) {
                    Ok(frame) => {
                        let preview = frame.downscale_for_preview(1280);
                        println!(
                            "  Captured {}x{} (preview {}x{}, is_hdr={}, label={}) in {:.1} ms",
                            frame.width,
                            frame.height,
                            preview.width,
                            preview.height,
                            frame.is_hdr,
                            frame.source_label,
                            t0.elapsed().as_secs_f32() * 1000.0
                        );
                        println!(
                            "  Stats: P50={:.1} | P95={:.1} | P99={:.1} | Max={:.1} nits | >SDR={:.2}% | WCG<0={:.2}%",
                            frame.stats.p50_nits,
                            frame.stats.p95_nits,
                            frame.stats.p99_nits,
                            frame.stats.max_nits,
                            frame.stats.pct_above_sdr_white,
                            frame.stats.pct_wide_gamut_negative,
                        );
                    }
                    Err(e) => {
                        eprintln!("  Capture error: {:#}", e);
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("Display enumeration error: {:#}", e);
        }
    }
}

fn main() {
    // Must be called before ANY Win32/DXGI/DisplayConfig call so IDXGIOutput5::DuplicateOutput1
    // is permitted to capture FP16 HDR desktop frames.
    unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
    }

    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--probe") {
        run_probe();
        return;
    }
    let start_hidden_in_tray = args.iter().any(|a| a == "--tray");

    let mut config = AppConfig::load();
    config.start_with_windows = tray::is_windows_startup_enabled();
    let (tray, tray_rx) = TrayController::spawn(config.shortcuts.clone());

    Application::new()
        .with_assets(IconsAssets)
        .run(move |cx: &mut App| {
            ui::launch(config, tray, tray_rx, start_hidden_in_tray, cx);
        });
}
