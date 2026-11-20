use crate::capture::capture_display_with_options;
use crate::clipboard::copy_bgra8_to_clipboard;
use crate::config::{AppConfig, BarPosition};
use crate::display::{DisplayInfo, WorkArea, display_index_at_cursor, enumerate_displays};
use crate::export::{save_hdr_exr, save_sdr_png};
use crate::frame::CapturedFrame;
use crate::tonemap::{GamutMapping, TonemapOperator, TonemapParams, tonemap_to_bgra8};
use crate::tray::{
    TrayController, configure_floating_window, configure_tooltip_window, get_window_rect,
    hide_window, move_tooltip_physical, move_window_physical, set_window_topmost, show_window,
};
use crate::ui::components::{SliderId, SliderSpec};
use crate::ui::gamebar::{GameBarView, PopoverView, TooltipView};
use crate::ui::snip::RegionSnipView;
use crate::ui::widgets::WidgetWindowView;
use chrono::Local;
use gpui::*;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WidgetId {
    Preview,
    Tonemap,
    Scope,
    Markup,
    Settings,
}

pub const GAMEBAR_HORIZ_WIDTH: f32 = 666.0;
pub const GAMEBAR_VERT_HEIGHT: f32 = 594.0;
pub const GAMEBAR_SHORT: f32 = 58.0;
pub const EDGE_MARGIN_LOGICAL: f32 = 22.0;
pub const PREVIEW_MAX_DIM: u32 = 640;

pub fn gamebar_logical_size(pos: BarPosition) -> (f32, f32) {
    if pos.is_vertical() {
        (GAMEBAR_SHORT, GAMEBAR_VERT_HEIGHT)
    } else {
        (GAMEBAR_HORIZ_WIDTH, GAMEBAR_SHORT)
    }
}

/// Computes Game Bar physical `(x, y)` anchored to `pos` (`Top`, `Bottom`, `Left`, `Right`)
/// inside the display's usable work area (`rcWork`, excluding the Windows Taskbar on any edge).
pub fn place_gamebar(pos: BarPosition, work: WorkArea, bw: i32, bh: i32, scale: f32) -> (i32, i32) {
    let edge_pad = (18.0 * scale).round() as i32;
    let side_pad = (EDGE_MARGIN_LOGICAL * scale).round() as i32;
    let (raw_x, raw_y) = match pos {
        BarPosition::Top => (work.left + (work.width - bw) / 2, work.top + edge_pad),
        BarPosition::Bottom => (
            work.left + (work.width - bw) / 2,
            work.bottom() - bh - edge_pad,
        ),
        BarPosition::Left => (work.left + edge_pad, work.top + (work.height - bh) / 2),
        BarPosition::Right => (
            work.right() - bw - edge_pad,
            work.top + (work.height - bh) / 2,
        ),
    };
    let margin_x = if pos.is_vertical() { edge_pad } else { side_pad };
    let margin_y = if pos.is_vertical() { side_pad } else { edge_pad };
    let (cx, _) = work.clamp_window(raw_x, raw_y, bw, bh, margin_x);
    let (_, cy) = work.clamp_window(cx, raw_y, bw, bh, margin_y);
    (cx, cy)
}

/// Computes floating widget physical `(x, y)` inside `work` (`rcWork`), keeping `EDGE_MARGIN_LOGICAL`
/// clearance from both monitor edges and the Windows Taskbar on any side, while also staying clear
/// of the Game Bar regardless of whether it is docked Top, Bottom, Left, or Right.
#[allow(clippy::too_many_arguments)]
pub fn place_widget(
    id: WidgetId,
    bar_pos: BarPosition,
    work: WorkArea,
    bar_rect: Option<(i32, i32, i32, i32)>,
    pw: i32,
    ph: i32,
    scale: f32,
    is_portrait: bool,
) -> (i32, i32) {
    let pad = (EDGE_MARGIN_LOGICAL * scale).round() as i32;
    let bar_gap = (18.0 * scale).round() as i32;

    let bar_on_work = bar_rect.filter(|&(bx, by, bw, bh)| {
        let cx = bx + bw / 2;
        let cy = by + bh / 2;
        cx >= work.left && cx <= work.right() && cy >= work.top && cy <= work.bottom()
    });

    // Shrink the effective placement area away from whichever edge hosts the Game Bar
    let mut safe = work;
    match bar_pos {
        BarPosition::Top => {
            let new_top = bar_on_work
                .map(|(_, by, _, bh)| (by + bh + bar_gap - pad).max(work.top))
                .unwrap_or(work.top + (76.0 * scale).round() as i32);
            let delta = (new_top - safe.top).max(0);
            safe.top = new_top;
            safe.height = (safe.height - delta).max(1);
        }
        BarPosition::Bottom => {
            let new_bottom = bar_on_work
                .map(|(_, by, _, _)| (by - bar_gap + pad).min(work.bottom()))
                .unwrap_or(work.bottom() - (76.0 * scale).round() as i32);
            safe.height = (new_bottom - safe.top).max(1);
        }
        BarPosition::Left => {
            let new_left = bar_on_work
                .map(|(bx, _, bw, _)| (bx + bw + bar_gap - pad).max(work.left))
                .unwrap_or(work.left + (76.0 * scale).round() as i32);
            let delta = (new_left - safe.left).max(0);
            safe.left = new_left;
            safe.width = (safe.width - delta).max(1);
        }
        BarPosition::Right => {
            let new_right = bar_on_work
                .map(|(bx, _, _, _)| (bx - bar_gap + pad).min(work.right()))
                .unwrap_or(work.right() - (76.0 * scale).round() as i32);
            safe.width = (new_right - safe.left).max(1);
        }
    }

    let top_y = safe.top + pad;
    let (raw_x, raw_y) = if is_portrait {
        match id {
            WidgetId::Preview => (safe.left + pad, top_y),
            WidgetId::Tonemap => (safe.right() - pw - pad, top_y),
            WidgetId::Scope => (safe.left + pad, top_y + (452.0 * scale).round() as i32),
            WidgetId::Markup => (safe.left + pad, top_y + (680.0 * scale).round() as i32),
            WidgetId::Settings => (safe.right() - pw - pad, safe.bottom() - ph - pad),
        }
    } else {
        match id {
            WidgetId::Preview => (safe.left + pad, top_y),
            WidgetId::Scope => (safe.left + pad, top_y + (392.0 * scale).round() as i32),
            WidgetId::Tonemap => (safe.right() - pw - pad, top_y),
            WidgetId::Markup => (safe.left + (safe.width - pw) / 2, top_y),
            WidgetId::Settings => (safe.right() - pw - pad, safe.bottom() - ph - pad),
        }
    };

    safe.clamp_window(raw_x, raw_y, pw, ph, pad)
}

/// Anchors a popover or floating tooltip adjacent to the Game Bar for any of the 4 dock edges:
/// - `Top` -> below bar (flips above if near bottom edge)
/// - `Bottom` -> above bar (flips below if near top edge)
/// - `Left` -> right of vertical bar (flips left if near right edge)
/// - `Right` -> left of vertical bar (flips right if near left edge)
#[allow(clippy::too_many_arguments)]
pub fn place_anchored_for_bar(
    bar_pos: BarPosition,
    work: WorkArea,
    bar_rect: (i32, i32, i32, i32),
    anchor_px: i32,
    center_on_anchor: bool,
    pw: i32,
    ph: i32,
    gap: i32,
    margin: i32,
) -> (i32, i32) {
    let (bx, by, bw, bh) = bar_rect;
    let (pref_x, pref_y) = match bar_pos {
        BarPosition::Top => {
            let x = if center_on_anchor {
                bx + anchor_px - pw / 2
            } else {
                bx + anchor_px
            };
            let below = by + bh + gap;
            let y = if below + ph + margin <= work.bottom() {
                below
            } else {
                by - ph - gap
            };
            (x, y)
        }
        BarPosition::Bottom => {
            let x = if center_on_anchor {
                bx + anchor_px - pw / 2
            } else {
                bx + anchor_px
            };
            let above = by - ph - gap;
            let y = if above >= work.top + margin {
                above
            } else {
                by + bh + gap
            };
            (x, y)
        }
        BarPosition::Left => {
            let y = if center_on_anchor {
                by + anchor_px - ph / 2
            } else {
                by + anchor_px
            };
            let right = bx + bw + gap;
            let x = if right + pw + margin <= work.right() {
                right
            } else {
                bx - pw - gap
            };
            (x, y)
        }
        BarPosition::Right => {
            let y = if center_on_anchor {
                by + anchor_px - ph / 2
            } else {
                by + anchor_px
            };
            let left = bx - pw - gap;
            let x = if left >= work.left + margin {
                left
            } else {
                bx + bw + gap
            };
            (x, y)
        }
    };
    work.clamp_window(pref_x, pref_y, pw, ph, margin)
}

impl WidgetId {
    pub fn default_size(self, portrait_frame: bool) -> (f32, f32) {
        match self {
            Self::Preview => (404.0, if portrait_frame { 438.0 } else { 378.0 }),
            Self::Tonemap => (312.0, 598.0),
            Self::Scope => (404.0, 214.0),
            Self::Markup => (272.0, 144.0),
            Self::Settings => (316.0, 576.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopoverKind {
    Display,
    HomeMenu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkupTool {
    Box,
    Arrow,
    Blur,
}

#[derive(Debug, Clone, Copy)]
pub struct MarkupShape {
    pub tool: MarkupTool,
    pub color: u32,
    pub u1: f32,
    pub v1: f32,
    pub u2: f32,
    pub v2: f32,
}

#[derive(Clone)]
pub struct ManagedWindow {
    pub handle: AnyWindowHandle,
    pub hwnd: isize,
    pub pinned: bool,
}

pub struct AppStore {
    pub config: AppConfig,
    pub displays: Vec<DisplayInfo>,
    pub selected_display: usize,

    pub frame_full: Arc<CapturedFrame>,
    pub frame_preview: Arc<CapturedFrame>,
    /// Shared tonemapped BGRA8 preview bytes for active operator (uploaded once per parameter change).
    pub preview_active_bgra: Arc<(u32, u32, Vec<u8>)>,
    pub preview_active_version: u64,
    /// Shared Windows/OBS Clip baseline preview bytes for zero-cost GPU A/B split curtain dragging.
    pub preview_baseline_bgra: Arc<(u32, u32, Vec<u8>)>,
    pub preview_baseline_version: u64,
    baseline_key: (u32, i32, bool, u64),
    frame_generation: u64,

    pub params: TonemapParams,
    pub split_on: bool,
    pub last_tonemap_ms: f32,
    pub is_capturing: bool,

    pub markup_tool: MarkupTool,
    pub markup_color: u32,
    pub shapes: Vec<MarkupShape>,

    pub show_nits_readout: bool,
    pub clock_str: String,
    pub status_line: Option<String>,

    // Independent floating windows registry
    pub bar_window: Option<ManagedWindow>,
    pub widget_windows: HashMap<WidgetId, ManagedWindow>,
    pub popover_window: Option<(PopoverKind, AnyWindowHandle, isize)>,
    pub tooltip_window: Option<(AnyWindowHandle, isize)>,
    pub tooltip_text: Option<&'static str>,
    pub tooltip_seq: u64,
    pub snip_window: Option<(AnyWindowHandle, isize)>,

    pub tray: TrayController,
    pub hidden_in_tray: Arc<AtomicBool>,
    pub allow_quit: Arc<AtomicBool>,
}

pub fn extract_hwnd(window: &Window) -> isize {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    if let Ok(handle) = HasWindowHandle::window_handle(window) {
        if let RawWindowHandle::Win32(win32) = handle.as_raw() {
            return win32.hwnd.get();
        }
    }
    0
}

pub fn current_clock_string() -> String {
    Local::now().format("%-I:%M %p").to_string()
}

impl AppStore {
    pub fn new(
        config: AppConfig,
        tray: TrayController,
        hidden_in_tray: Arc<AtomicBool>,
        allow_quit: Arc<AtomicBool>,
    ) -> Self {
        let displays = enumerate_displays().unwrap_or_default();
        let selected_display = displays
            .iter()
            .position(|d| d.is_hdr)
            .or_else(|| displays.iter().position(|d| d.is_primary))
            .unwrap_or(0);

        let default_sdr = displays
            .get(selected_display)
            .map(|d| d.sdr_white_nits)
            .unwrap_or(280.0);

        let synth = Arc::new(CapturedFrame::synthetic_hdr_test_pattern(
            960,
            540,
            default_sdr,
        ));
        let preview = Arc::new(synth.downscale_for_preview(PREVIEW_MAX_DIM));

        let params = TonemapParams {
            operator: TonemapOperator::HybridSdrSafe,
            sdr_white_nits: default_sdr,
            max_luminance_nits: 1200.0,
            ..Default::default()
        };

        let mut store = Self {
            config,
            displays,
            selected_display,
            frame_full: synth,
            frame_preview: preview,
            preview_active_bgra: Arc::new((1, 1, vec![0, 0, 0, 255])),
            preview_active_version: 0,
            preview_baseline_bgra: Arc::new((1, 1, vec![0, 0, 0, 255])),
            preview_baseline_version: 0,
            baseline_key: (0, 0, false, 0),
            frame_generation: 1,
            params,
            split_on: true,
            last_tonemap_ms: 0.0,
            is_capturing: false,
            markup_tool: MarkupTool::Box,
            markup_color: 0xef4444,
            shapes: Vec::new(),
            show_nits_readout: false,
            clock_str: current_clock_string(),
            status_line: None,
            bar_window: None,
            widget_windows: HashMap::new(),
            popover_window: None,
            tooltip_window: None,
            tooltip_text: None,
            tooltip_seq: 0,
            snip_window: None,
            tray,
            hidden_in_tray,
            allow_quit,
        };
        store.recompute_preview();
        store
    }

    pub fn default_peak_nits(&self) -> f32 {
        let suggested = self
            .frame_full
            .stats
            .max_nits
            .max(self.frame_full.monitor_max_nits)
            .max(self.frame_full.sdr_white_nits * 2.0)
            .clamp(300.0, 4000.0);
        (suggested / 50.0).round() * 50.0
    }

    pub fn recompute_preview(&mut self) {
        let start = Instant::now();
        let bgra_active = tonemap_to_bgra8(&self.frame_preview, &self.params);
        self.last_tonemap_ms = start.elapsed().as_secs_f32() * 1000.0;
        self.preview_active_bgra = Arc::new((
            self.frame_preview.width,
            self.frame_preview.height,
            bgra_active,
        ));
        self.preview_active_version = self.preview_active_version.wrapping_add(1);

        // Only recompute the Windows Clip baseline texture when SDR White, Exposure, Gamut, or frame changes
        let key = (
            self.params.sdr_white_nits.to_bits(),
            (self.params.exposure_ev * 100.0).round() as i32,
            self.params.gamut_mapping == GamutMapping::SoftLuminance,
            self.frame_generation,
        );
        if key != self.baseline_key {
            let base_params = TonemapParams {
                operator: TonemapOperator::WindowsClip,
                show_hdr_heatmap: false,
                ..self.params
            };
            let bgra_base = tonemap_to_bgra8(&self.frame_preview, &base_params);
            self.preview_baseline_bgra = Arc::new((
                self.frame_preview.width,
                self.frame_preview.height,
                bgra_base,
            ));
            self.preview_baseline_version = self.preview_baseline_version.wrapping_add(1);
            self.baseline_key = key;
        }
    }

    pub fn set_captured_frame(&mut self, full: Arc<CapturedFrame>, preview: Arc<CapturedFrame>) {
        self.params.sdr_white_nits = full.sdr_white_nits;
        self.frame_full = full;
        self.frame_preview = preview;
        self.frame_generation = self.frame_generation.wrapping_add(1);
        self.params.max_luminance_nits = self.default_peak_nits();
        self.shapes.clear();
        self.recompute_preview();
    }

    pub fn reset_tonemap_defaults(&mut self) {
        self.params = TonemapParams {
            operator: TonemapOperator::HybridSdrSafe,
            sdr_white_nits: self.frame_full.sdr_white_nits,
            exposure_ev: 0.0,
            max_luminance_nits: self.default_peak_nits(),
            shoulder_start: 0.78,
            highlight_desat: 0.35,
            gamut_mapping: GamutMapping::SoftLuminance,
            show_hdr_heatmap: false,
        };
        self.recompute_preview();
        self.status_line = Some("Tonemap reset to default".to_string());
    }

    pub fn slider_spec(&self, id: SliderId) -> SliderSpec {
        match id {
            SliderId::SdrWhite => SliderSpec {
                id,
                label: "SDR White",
                min: 80.0,
                max: 500.0,
                step: 10.0,
                default_value: self.frame_full.sdr_white_nits,
                value: self.params.sdr_white_nits,
                dimmed: false,
            },
            SliderId::Exposure => SliderSpec {
                id,
                label: "Exposure",
                min: -2.0,
                max: 2.0,
                step: 0.05,
                default_value: 0.0,
                value: self.params.exposure_ev,
                dimmed: false,
            },
            SliderId::PeakLum => SliderSpec {
                id,
                label: "Peak Lum",
                min: 300.0,
                max: 4000.0,
                step: 50.0,
                default_value: self.default_peak_nits(),
                value: self.params.max_luminance_nits,
                dimmed: !self.params.operator.uses_max_luminance(),
            },
            SliderId::Shoulder => SliderSpec {
                id,
                label: "Shoulder",
                min: 0.50,
                max: 0.94,
                step: 0.02,
                default_value: 0.78,
                value: self.params.shoulder_start,
                dimmed: !self.params.operator.uses_shoulder(),
            },
            SliderId::Desat => SliderSpec {
                id,
                label: "Desat",
                min: 0.0,
                max: 1.0,
                step: 0.05,
                default_value: 0.35,
                value: self.params.highlight_desat,
                dimmed: false,
            },
        }
    }

    pub fn apply_slider_value(&mut self, id: SliderId, val: f32) -> bool {
        let changed = match id {
            SliderId::SdrWhite => {
                let c = (self.params.sdr_white_nits - val).abs() > 1e-4;
                self.params.sdr_white_nits = val;
                c
            }
            SliderId::Exposure => {
                let c = (self.params.exposure_ev - val).abs() > 1e-4;
                self.params.exposure_ev = val;
                c
            }
            SliderId::PeakLum => {
                let c = (self.params.max_luminance_nits - val).abs() > 1e-4;
                self.params.max_luminance_nits = val;
                c
            }
            SliderId::Shoulder => {
                let c = (self.params.shoulder_start - val).abs() > 1e-4;
                self.params.shoulder_start = val;
                c
            }
            SliderId::Desat => {
                let c = (self.params.highlight_desat - val).abs() > 1e-4;
                self.params.highlight_desat = val;
                c
            }
        };
        if changed {
            self.recompute_preview();
        }
        changed
    }

    pub fn reset_single_slider(&mut self, id: SliderId) {
        let val = self.slider_spec(id).default_value;
        self.apply_slider_value(id, val);
    }

    pub fn nudge_slider(&mut self, id: SliderId, delta: f32) {
        let spec = self.slider_spec(id);
        let next = spec.snap(spec.value + delta);
        self.apply_slider_value(id, next);
    }

    pub fn is_widget_open(&self, id: WidgetId) -> bool {
        self.widget_windows.contains_key(&id)
    }

    pub fn is_widget_pinned(&self, id: WidgetId) -> bool {
        self.widget_windows.get(&id).map(|w| w.pinned).unwrap_or(true)
    }

    pub fn sync_hotkeys(&self) {
        self.tray.update_shortcuts(self.config.shortcuts.clone());
    }

    pub fn refresh_displays(&mut self) {
        match enumerate_displays() {
            Ok(list) => {
                self.displays = list;
                if self.selected_display >= self.displays.len() {
                    self.selected_display = 0;
                }
                self.status_line = Some(format!("{} display(s) detected", self.displays.len()));
            }
            Err(e) => self.status_line = Some(format!("Display error: {}", e)),
        }
    }

    pub fn select_display_at_cursor(&mut self) {
        if let Some(idx) = display_index_at_cursor(&self.displays) {
            self.selected_display = idx;
        }
    }

    pub fn copy_to_clipboard(&mut self) {
        let full_bgra = tonemap_to_bgra8(&self.frame_full, &self.params);
        match copy_bgra8_to_clipboard(self.frame_full.width, self.frame_full.height, &full_bgra) {
            Ok(()) => {
                self.status_line = Some(format!(
                    "Copied {}×{} · {}",
                    self.frame_full.width,
                    self.frame_full.height,
                    self.params.operator.short_label()
                ));
            }
            Err(e) => self.status_line = Some(format!("Copy failed: {}", e)),
        }
    }

    pub fn save_png(&mut self) {
        let full_bgra = tonemap_to_bgra8(&self.frame_full, &self.params);
        match save_sdr_png(
            self.frame_full.width,
            self.frame_full.height,
            &full_bgra,
            self.params.operator.short_label(),
        ) {
            Ok(path) => {
                self.status_line = Some(format!(
                    "Saved {}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ));
            }
            Err(e) => self.status_line = Some(format!("Save PNG failed: {}", e)),
        }
    }

    pub fn save_exr(&mut self) {
        match save_hdr_exr(&self.frame_full) {
            Ok(path) => {
                self.status_line = Some(format!(
                    "Saved {}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ));
            }
            Err(e) => self.status_line = Some(format!("Save EXR failed: {}", e)),
        }
    }

    pub fn apply_post_capture_actions(&mut self) {
        let mut notes = Vec::new();
        if self.config.auto_copy {
            let full_bgra = tonemap_to_bgra8(&self.frame_full, &self.params);
            if copy_bgra8_to_clipboard(self.frame_full.width, self.frame_full.height, &full_bgra)
                .is_ok()
            {
                notes.push("Copied");
            }
        }
        if self.config.auto_save_png {
            let full_bgra = tonemap_to_bgra8(&self.frame_full, &self.params);
            if save_sdr_png(
                self.frame_full.width,
                self.frame_full.height,
                &full_bgra,
                self.params.operator.short_label(),
            )
            .is_ok()
            {
                notes.push("PNG");
            }
        }
        let suffix = if notes.is_empty() {
            String::new()
        } else {
            format!(" · {}", notes.join(" + "))
        };
        self.status_line = Some(format!(
            "{}×{} ({}){}",
            self.frame_full.width,
            self.frame_full.height,
            if self.frame_full.is_hdr { "FP16" } else { "SDR" },
            suffix
        ));
    }

    fn all_floating_hwnds(&self) -> Vec<(isize, bool)> {
        let mut out = Vec::new();
        if let Some(bar) = &self.bar_window {
            out.push((bar.hwnd, true));
        }
        for w in self.widget_windows.values() {
            out.push((w.hwnd, w.pinned));
        }
        if let Some((_, _, hwnd)) = self.popover_window {
            out.push((hwnd, true));
        }
        out
    }
}

// ── Independent Window Lifecycle Helpers (operate on Entity<AppStore> + &mut App) ──

pub fn hide_floating_tooltip(store: &Entity<AppStore>, cx: &mut App) {
    let tip = store.update(cx, |s, cx| {
        s.tooltip_seq = s.tooltip_seq.wrapping_add(1);
        s.tooltip_text = None;
        let w = s.tooltip_window.take();
        if w.is_some() {
            cx.notify();
        }
        w
    });
    if let Some((handle, _)) = tip {
        let _ = cx.update_window(handle, |_, window, _| {
            window.remove_window();
        });
    }
}

pub fn show_floating_tooltip(
    store: &Entity<AppStore>,
    text: &'static str,
    anchor_logical: f32,
    scale: f32,
    cx: &mut App,
) {
    let (bar_window, existing, display, bar_pos) = {
        let s = store.read(cx);
        if s.hidden_in_tray.load(Ordering::SeqCst) || s.popover_window.is_some() {
            return;
        }
        (
            s.bar_window.clone(),
            s.tooltip_window,
            s.displays.get(s.selected_display).cloned(),
            s.config.bar_position,
        )
    };

    let Some(bar) = bar_window else {
        return;
    };
    let Some(bar_rect) = get_window_rect(bar.hwnd) else {
        return;
    };

    let scale = scale.max(1.0);

    let tip_w_logical = ((text.len() as f32) * 7.6 + 26.0).ceil().max(72.0);
    let tip_h_logical = 28.0f32;
    let pw = (tip_w_logical * scale).round() as i32;
    let ph = (tip_h_logical * scale).round() as i32;

    let anchor_px = (anchor_logical * scale).round() as i32;
    let gap = (6.0 * scale).round() as i32;
    let margin = (8.0 * scale).round() as i32;
    let (tx, ty) = if let Some(d) = &display {
        let work = d.live_work_area();
        place_anchored_for_bar(bar_pos, work, bar_rect, anchor_px, true, pw, ph, gap, margin)
    } else {
        (bar_rect.0 + anchor_px - pw / 2, bar_rect.1 + bar_rect.3 + gap)
    };

    let seq = store.update(cx, |s, _| {
        s.tooltip_seq = s.tooltip_seq.wrapping_add(1);
        s.tooltip_seq
    });

    if let Some((handle, hwnd)) = existing {
        // Defer SetWindowPos out of the current `on_hover` borrow so WM_SIZE -> GPUI's resize callback
        // can cleanly re-enter `App` and update `Window.viewport_size` to the new width before painting!
        let store_clone = store.clone();
        cx.spawn(async move |cx| {
            let still_current = cx
                .update(|cx| {
                    let s = store_clone.read(cx);
                    s.tooltip_seq == seq && s.tooltip_window.map(|(_, h)| h) == Some(hwnd)
                })
                .unwrap_or(false);
            if !still_current {
                return;
            }
            move_tooltip_physical(hwnd, tx, ty, pw, ph);
            let _ = cx.update(|cx| {
                store_clone.update(cx, |s, cx| {
                    if s.tooltip_seq == seq {
                        s.tooltip_text = Some(text);
                        cx.notify();
                    }
                });
                let _ = cx.update_window(handle, |_, window, _| {
                    window.refresh();
                });
            });
        })
        .detach();
        return;
    }

    store.update(cx, |s, cx| {
        s.tooltip_text = Some(text);
        cx.notify();
    });

    let bounds = Bounds::new(
        point(px(0.0), px(0.0)),
        size(px(tip_w_logical), px(tip_h_logical)),
    );
    let store_clone = store.clone();

    let _ = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                appears_transparent: false,
                ..Default::default()
            }),
            kind: WindowKind::PopUp,
            window_background: WindowBackgroundAppearance::Opaque,
            window_decorations: Some(WindowDecorations::Client),
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            focus: false,
            show: true,
            ..Default::default()
        },
        move |window, cx| {
            let hwnd = extract_hwnd(window);
            configure_tooltip_window(hwnd);
            move_tooltip_physical(hwnd, tx, ty, pw, ph);

            let any_h = window.window_handle();
            store_clone.update(cx, |s, cx| {
                s.tooltip_window = Some((any_h, hwnd));
                cx.notify();
            });

            cx.new(|cx| TooltipView::new(store_clone, cx))
        },
    );
}

pub fn close_popover(store: &Entity<AppStore>, cx: &mut App) {
    let pop = store.update(cx, |s, cx| {
        let p = s.popover_window.take();
        cx.notify();
        p
    });
    if let Some((_, handle, _)) = pop {
        let _ = cx.update_window(handle, |_, window, _| {
            window.remove_window();
        });
    }
}

pub fn toggle_popover(store: &Entity<AppStore>, kind: PopoverKind, cx: &mut App) {
    hide_floating_tooltip(store, cx);
    let existing = store.read(cx).popover_window.map(|(k, _, _)| k);
    close_popover(store, cx);
    if existing == Some(kind) {
        return;
    }

    let (display_count, bar_hwnd, display, bar_pos) = {
        let s = store.read(cx);
        (
            s.displays.len().max(1),
            s.bar_window.as_ref().map(|b| b.hwnd).unwrap_or(0),
            s.displays.get(s.selected_display).cloned(),
            s.config.bar_position,
        )
    };
    let (w, h) = match kind {
        PopoverKind::Display => (320.0, 62.0 + (display_count as f32) * 38.0),
        PopoverKind::HomeMenu => (276.0, 316.0),
    };

    let bar_rect = get_window_rect(bar_hwnd);

    let bounds = Bounds::new(point(px(200.0), px(90.0)), size(px(w), px(h)));
    let store_clone = store.clone();

    if let Ok(handle) = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                appears_transparent: false,
                ..Default::default()
            }),
            kind: WindowKind::PopUp,
            window_background: WindowBackgroundAppearance::Opaque,
            window_decorations: Some(WindowDecorations::Client),
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            focus: true,
            show: true,
            ..Default::default()
        },
        move |window, cx| {
            let hwnd = extract_hwnd(window);
            configure_floating_window(hwnd, true);
            if let Some(br) = bar_rect {
                let scale = window.scale_factor();
                let pw = (w * scale).round() as i32;
                let ph = (h * scale).round() as i32;
                let anchor_px = if kind == PopoverKind::Display {
                    0
                } else {
                    (62.0 * scale).round() as i32
                };
                let gap = (8.0 * scale).round() as i32;
                let margin = (12.0 * scale).round() as i32;
                let (px_x, px_y) = if let Some(d) = &display {
                    let work = d.live_work_area();
                    place_anchored_for_bar(bar_pos, work, br, anchor_px, false, pw, ph, gap, margin)
                } else {
                    (br.0 + anchor_px, br.1 + br.3 + gap)
                };
                move_window_physical(hwnd, px_x, px_y, pw, ph, true);
            }
            let any_h = window.window_handle();
            store_clone.update(cx, |s, cx| {
                s.popover_window = Some((kind, any_h, hwnd));
                cx.notify();
            });
            cx.new(|cx| PopoverView::new(store_clone, kind, cx))
        },
    ) {
        let _ = handle;
    }
}

pub fn open_widget(store: &Entity<AppStore>, id: WidgetId, cx: &mut App) {
    if let Some(existing) = store.read(cx).widget_windows.get(&id).cloned() {
        show_window(existing.hwnd, existing.pinned, true);
        return;
    }

    let (portrait, bar_hwnd, display, bar_pos) = {
        let s = store.read(cx);
        let p = s.frame_full.height > s.frame_full.width;
        let bh = s.bar_window.as_ref().map(|b| b.hwnd).unwrap_or(0);
        let d = s.displays.get(s.selected_display).cloned();
        (p, bh, d, s.config.bar_position)
    };

    let (w, h) = id.default_size(portrait);
    let bar_rect = get_window_rect(bar_hwnd);
    let bounds = Bounds::new(point(px(60.0), px(110.0)), size(px(w), px(h)));
    let store_clone = store.clone();

    let _ = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
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
            focus: true,
            show: true,
            ..Default::default()
        },
        move |window, cx| {
            let hwnd = extract_hwnd(window);
            let pinned = matches!(id, WidgetId::Tonemap);
            configure_floating_window(hwnd, pinned);

            // Smart initial physical placement inside the display's live work area
            // (excluding the Windows Taskbar on Left, Right, Top, or Bottom — landscape or portrait!)
            let scale = window.scale_factor();
            let pw = (w * scale).round() as i32;
            let ph = (h * scale).round() as i32;

            let (tx, ty) = if let Some(d) = &display {
                let work = d.live_work_area();
                place_widget(id, bar_pos, work, bar_rect, pw, ph, scale, d.is_portrait())
            } else {
                (60, 110)
            };

            move_window_physical(hwnd, tx, ty, pw, ph, pinned);

            let any_h = window.window_handle();
            store_clone.update(cx, |s, cx| {
                s.widget_windows.insert(
                    id,
                    ManagedWindow {
                        handle: any_h,
                        hwnd,
                        pinned,
                    },
                );
                cx.notify();
            });

            // Closing via OS (Alt+F4) cleanly removes widget from registry
            let store_close = store_clone.clone();
            window.on_window_should_close(cx, move |_window, cx| {
                store_close.update(cx, |s, cx| {
                    s.widget_windows.remove(&id);
                    cx.notify();
                });
                true
            });

            cx.new(|cx| WidgetWindowView::new(store_clone, id, hwnd, window, cx))
        },
    );
}

pub fn close_widget(store: &Entity<AppStore>, id: WidgetId, cx: &mut App) {
    let removed = store.update(cx, |s, cx| {
        let w = s.widget_windows.remove(&id);
        cx.notify();
        w
    });
    if let Some(w) = removed {
        let _ = cx.update_window(w.handle, |_, window, _| {
            window.remove_window();
        });
    }
}

pub fn toggle_widget(store: &Entity<AppStore>, id: WidgetId, cx: &mut App) {
    if store.read(cx).is_widget_open(id) {
        close_widget(store, id, cx);
    } else {
        open_widget(store, id, cx);
    }
}

pub fn toggle_widget_pin(store: &Entity<AppStore>, id: WidgetId, cx: &mut App) {
    store.update(cx, |s, cx| {
        if let Some(w) = s.widget_windows.get_mut(&id) {
            w.pinned = !w.pinned;
            set_window_topmost(w.hwnd, w.pinned);
            cx.notify();
        }
    });
}

pub fn move_gamebar_to_display(store: &Entity<AppStore>, idx: usize, cx: &mut App) {
    hide_floating_tooltip(store, cx);
    let info = store.update(cx, |s, cx| {
        if idx < s.displays.len() {
            s.selected_display = idx;
            let d = s.displays[idx].clone();
            let bar = s.bar_window.clone();
            let pos = s.config.bar_position;
            s.status_line = Some(format!("Display: {} ({}×{})", d.friendly_name, d.width, d.height));
            cx.notify();
            Some((d, bar, pos))
        } else {
            None
        }
    });

    if let Some((d, Some(bar), pos)) = info {
        let scale = cx
            .update_window(bar.handle, |_, window, _| window.scale_factor())
            .unwrap_or(1.0);
        let (lw, lh) = gamebar_logical_size(pos);
        let bw = (lw * scale).round() as i32;
        let bh = (lh * scale).round() as i32;
        let work = d.live_work_area();
        let (bx, by) = place_gamebar(pos, work, bw, bh, scale);
        let handle = bar.handle;
        let hwnd = bar.hwnd;
        cx.spawn(async move |cx| {
            move_window_physical(hwnd, bx, by, bw, bh, true);
            let _ = cx.update_window(handle, |_, window, _| window.refresh());
        })
        .detach();
    }
}

/// Switches the Game Bar dock edge (`Top`, `Bottom`, `Left`, `Right`), toggling between horizontal
/// (`666×58`) and vertical (`58×666`) orientation live and persisting to `config.json`.
pub fn set_bar_position(store: &Entity<AppStore>, pos: BarPosition, cx: &mut App) {
    hide_floating_tooltip(store, cx);
    close_popover(store, cx);
    let info = store.update(cx, |s, cx| {
        s.config.bar_position = pos;
        s.config.save();
        s.status_line = Some(format!("Game Bar: {}", pos.label()));
        let d = s.displays.get(s.selected_display).cloned();
        let bar = s.bar_window.clone();
        cx.notify();
        (d, bar)
    });

    if let (Some(d), Some(bar)) = info {
        let scale = cx
            .update_window(bar.handle, |_, window, _| window.scale_factor())
            .unwrap_or(1.0);
        let (lw, lh) = gamebar_logical_size(pos);
        let bw = (lw * scale).round() as i32;
        let bh = (lh * scale).round() as i32;
        let work = d.live_work_area();
        let (bx, by) = place_gamebar(pos, work, bw, bh, scale);
        let handle = bar.handle;
        let hwnd = bar.hwnd;
        // Defer SetWindowPos out of the current click handler borrow so WM_SIZE cleanly
        // updates GPUI's `Window.viewport_size` & DirectX swapchain (666x58 <-> 58x666).
        cx.spawn(async move |cx| {
            move_window_physical(hwnd, bx, by, bw, bh, true);
            let _ = cx.update_window(handle, |_, window, _| window.refresh());
        })
        .detach();
    }
}

pub fn hide_all_to_tray(store: &Entity<AppStore>, cx: &mut App) {
    hide_floating_tooltip(store, cx);
    close_popover(store, cx);
    close_snip_window(store, cx);

    // Synchronously release GPU preview textures and cancel any hover tooltip timers
    // before hiding all windows so System Tray idle uses 0% CPU and minimal VRAM.
    let (hwnds, bar_handle, preview_handle) = store.update(cx, |s, cx| {
        s.hidden_in_tray.store(true, Ordering::SeqCst);
        let bh = s.bar_window.as_ref().map(|w| w.handle);
        let ph = s.widget_windows.get(&WidgetId::Preview).map(|w| w.handle);
        cx.notify();
        (s.all_floating_hwnds(), bh, ph)
    });

    if let Some(handle) = bar_handle.and_then(|h| h.downcast::<GameBarView>()) {
        let _ = handle.update(cx, |view, _, cx| {
            view.clear_hover_tip(cx);
        });
    }

    if let Some(handle) = preview_handle.and_then(|h| h.downcast::<WidgetWindowView>()) {
        let _ = handle.update(cx, |view, window, _| {
            view.release_gpu_textures(window);
        });
    }

    for (hwnd, _) in hwnds {
        hide_window(hwnd);
    }
}

pub fn show_all_from_tray(store: &Entity<AppStore>, cx: &mut App) {
    let (hwnds, bar, widgets, display, bar_pos, preview_handle) = store.update(cx, |s, cx| {
        s.hidden_in_tray.store(false, Ordering::SeqCst);
        s.clock_str = current_clock_string();
        let bar = s.bar_window.clone();
        let widgets: Vec<ManagedWindow> = s.widget_windows.values().cloned().collect();
        let display = s.displays.get(s.selected_display).cloned();
        let pos = s.config.bar_position;
        let ph = s.widget_windows.get(&WidgetId::Preview).map(|w| w.handle);
        cx.notify();
        (s.all_floating_hwnds(), bar, widgets, display, pos, ph)
    });

    // Re-verify live Windows Taskbar work area (`rcWork`) every single time we restore from tray
    // so neither the Game Bar nor any open widget can ever be partially covered if the taskbar moved!
    if let Some(d) = &display {
        let work = d.live_work_area();

        if let Some(bar_win) = &bar {
            let scale = cx
                .update_window(bar_win.handle, |_, window, _| window.scale_factor())
                .unwrap_or(1.0);
            let (lw, lh) = gamebar_logical_size(bar_pos);
            let bw = (lw * scale).round() as i32;
            let bh = (lh * scale).round() as i32;
            let (bx, by) = place_gamebar(bar_pos, work, bw, bh, scale);
            move_window_physical(bar_win.hwnd, bx, by, bw, bh, true);
        }

        for w in &widgets {
            if let Some((wx, wy, ww, wh)) = get_window_rect(w.hwnd) {
                let scale = cx
                    .update_window(w.handle, |_, window, _| window.scale_factor())
                    .unwrap_or(1.0);
                let pad = (EDGE_MARGIN_LOGICAL * scale).round() as i32;
                let (cx_w, cy_w) = work.clamp_window(wx, wy, ww, wh, pad);
                if cx_w != wx || cy_w != wy {
                    move_window_physical(w.hwnd, cx_w, cy_w, ww, wh, w.pinned);
                }
            }
        }
    }

    for (hwnd, topmost) in hwnds {
        show_window(hwnd, topmost, false);
    }
    if let Some(handle) = preview_handle {
        let _ = cx.update_window(handle, |_, window, _| {
            window.refresh();
        });
    }
    if let Some(bar_win) = bar {
        show_window(bar_win.hwnd, true, true);
    }
    cx.activate(true);
}

pub fn quit_application(store: &Entity<AppStore>, cx: &mut App) {
    store.update(cx, |s, _| {
        s.allow_quit.store(true, Ordering::SeqCst);
        s.tray.shutdown();
    });
    cx.quit();
}

pub fn close_snip_window(store: &Entity<AppStore>, cx: &mut App) {
    let snip = store.update(cx, |s, cx| {
        let w = s.snip_window.take();
        cx.notify();
        w
    });
    if let Some((handle, _)) = snip {
        let _ = cx.update_window(handle, |_, window, _| {
            window.remove_window();
        });
    }
}

/// Captures the target display in background (excluding floating HUD windows), then either opens
/// the full-monitor `RegionSnipView` on that exact monitor (landscape or portrait!) or finishes immediately.
pub fn trigger_capture(
    store: &Entity<AppStore>,
    delay_secs: u64,
    enter_region_snip: bool,
    cx: &mut App,
) {
    if store.read(cx).is_capturing {
        return;
    }
    hide_floating_tooltip(store, cx);
    close_popover(store, cx);
    close_snip_window(store, cx);

    let (display, capture_cursor, was_hidden, hwnds_to_hide) = {
        let s = store.read(cx);
        let Some(d) = s.displays.get(s.selected_display).cloned() else {
            return;
        };
        (
            d,
            s.config.capture_cursor,
            s.hidden_in_tray.load(Ordering::SeqCst),
            s.all_floating_hwnds(),
        )
    };

    store.update(cx, |s, cx| {
        s.is_capturing = true;
        cx.notify();
    });

    if !was_hidden {
        for (hwnd, _) in &hwnds_to_hide {
            hide_window(*hwnd);
        }
    }

    let store_clone = store.clone();
    cx.spawn(async move |cx| {
        let wait_ms = if delay_secs > 0 { delay_secs * 1000 } else { 150 };
        cx.background_executor()
            .timer(Duration::from_millis(wait_ms))
            .await;

        let display_for_cap = display.clone();
        let result = cx
            .background_executor()
            .spawn(async move {
                let full = capture_display_with_options(&display_for_cap, capture_cursor)?;
                let preview = full.downscale_for_preview(PREVIEW_MAX_DIM);
                Ok::<_, anyhow::Error>((full, preview))
            })
            .await;

        let _ = cx.update(|cx| {
            store_clone.update(cx, |s, cx| {
                s.is_capturing = false;
                cx.notify();
            });

            match result {
                Ok((captured_full, captured_preview)) => {
                    let full_arc = Arc::new(captured_full);
                    let prev_arc = Arc::new(captured_preview);

                    if enter_region_snip {
                        open_region_snip_window(&store_clone, display, full_arc, cx);
                    } else {
                        store_clone.update(cx, |s, cx| {
                            s.set_captured_frame(full_arc, prev_arc);
                            s.apply_post_capture_actions();
                            cx.notify();
                        });
                        show_all_from_tray(&store_clone, cx);
                        open_widget(&store_clone, WidgetId::Preview, cx);
                    }
                }
                Err(e) => {
                    store_clone.update(cx, |s, cx| {
                        s.status_line = Some(format!("Capture failed: {}", e));
                        cx.notify();
                    });
                    if !was_hidden {
                        show_all_from_tray(&store_clone, cx);
                    }
                }
            }
        });
    })
    .detach();
}

fn open_region_snip_window(
    store: &Entity<AppStore>,
    display: DisplayInfo,
    captured_full: Arc<CapturedFrame>,
    cx: &mut App,
) {
    let params = {
        let s = store.read(cx);
        let mut p = s.params;
        p.sdr_white_nits = captured_full.sdr_white_nits;
        p
    };
    let frozen_bgra = tonemap_to_bgra8(&captured_full, &params);
    let bounds = Bounds::new(
        point(px(0.0), px(0.0)),
        size(px(display.width as f32), px(display.height as f32)),
    );
    let store_clone = store.clone();

    let _ = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                appears_transparent: false,
                ..Default::default()
            }),
            kind: WindowKind::PopUp,
            window_background: WindowBackgroundAppearance::Opaque,
            window_decorations: Some(WindowDecorations::Client),
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            focus: true,
            show: true,
            ..Default::default()
        },
        move |window, cx| {
            let hwnd = extract_hwnd(window);
            configure_floating_window(hwnd, true);
            move_window_physical(
                hwnd,
                display.left,
                display.top,
                display.width as i32,
                display.height as i32,
                true,
            );
            show_window(hwnd, true, true);

            let any_h = window.window_handle();
            store_clone.update(cx, |s, cx| {
                s.snip_window = Some((any_h, hwnd));
                cx.notify();
            });

            cx.new(|cx| RegionSnipView::new(store_clone, captured_full, frozen_bgra, window, cx))
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_inside_work_area(
        label: &str,
        work: WorkArea,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        margin: i32,
    ) {
        assert!(
            x >= work.left + margin,
            "{label}: left edge {x} violated work.left+margin ({}+{})",
            work.left,
            margin
        );
        assert!(
            x + w <= work.right() - margin,
            "{label}: right edge {} violated work.right-margin ({}-{})",
            x + w,
            work.right(),
            margin
        );
        assert!(
            y >= work.top + margin,
            "{label}: top edge {y} violated work.top+margin ({}+{})",
            work.top,
            margin
        );
        assert!(
            y + h <= work.bottom() - margin,
            "{label}: bottom edge {} violated work.bottom-margin ({}-{})",
            y + h,
            work.bottom(),
            margin
        );
    }

    #[core::prelude::v1::test]
    fn placement_respects_all_four_taskbar_edges_landscape_and_portrait() {
        let setups: &[(&str, WorkArea, bool)] = &[
            // Landscape 2560x1440 with 62px taskbar on Left, Right, Top, Bottom
            ("landscape_taskbar_left", WorkArea { left: 62, top: 0, width: 2560 - 62, height: 1440 }, false),
            ("landscape_taskbar_right", WorkArea { left: 0, top: 0, width: 2560 - 62, height: 1440 }, false),
            ("landscape_taskbar_top", WorkArea { left: 0, top: 48, width: 2560, height: 1440 - 48 }, false),
            ("landscape_taskbar_bottom", WorkArea { left: 0, top: 0, width: 2560, height: 1440 - 48 }, false),
            // Portrait 1080x1920 @ (2560, -481) with taskbar on Left, Right, Top, Bottom
            ("portrait_taskbar_left", WorkArea { left: 2560 + 62, top: -481, width: 1080 - 62, height: 1920 }, true),
            ("portrait_taskbar_right", WorkArea { left: 2560, top: -481, width: 1080 - 62, height: 1920 }, true),
            ("portrait_taskbar_top", WorkArea { left: 2560, top: -481 + 48, width: 1080, height: 1920 - 48 }, true),
            ("portrait_taskbar_bottom", WorkArea { left: 2560, top: -481, width: 1080, height: 1920 - 48 }, true),
        ];

        let widgets = [
            WidgetId::Preview,
            WidgetId::Tonemap,
            WidgetId::Scope,
            WidgetId::Markup,
            WidgetId::Settings,
        ];

        for &(name, work, is_portrait) in setups {
            for &bar_pos in BarPosition::ALL {
                for scale in [1.0f32, 1.25f32] {
                    let (lw, lh) = gamebar_logical_size(bar_pos);
                    let bw = (lw * scale).round() as i32;
                    let bh = (lh * scale).round() as i32;
                    let (bx, by) = place_gamebar(bar_pos, work, bw, bh, scale);
                    let min_pad = (18.0 * scale).round() as i32;
                    assert_inside_work_area(
                        &format!("{name}_{bar_pos:?}_bar@{scale}"),
                        work,
                        bx,
                        by,
                        bw,
                        bh,
                        min_pad,
                    );

                    let pad = (EDGE_MARGIN_LOGICAL * scale).round() as i32;
                    for id in widgets {
                        let (w, h) = id.default_size(is_portrait);
                        let pw = (w * scale).round() as i32;
                        let ph = (h * scale).round() as i32;
                        let (wx, wy) = place_widget(
                            id,
                            bar_pos,
                            work,
                            Some((bx, by, bw, bh)),
                            pw,
                            ph,
                            scale,
                            is_portrait,
                        );
                        assert_inside_work_area(
                            &format!("{name}_{bar_pos:?}_{id:?}@{scale}"),
                            work,
                            wx,
                            wy,
                            pw,
                            ph,
                            pad,
                        );
                    }

                    // Popover stays strictly inside work area for all 4 bar dock positions
                    let pop_w = (276.0 * scale).round() as i32;
                    let pop_h = (316.0 * scale).round() as i32;
                    let (px_x, px_y) = place_anchored_for_bar(
                        bar_pos,
                        work,
                        (bx, by, bw, bh),
                        (62.0 * scale).round() as i32,
                        false,
                        pop_w,
                        pop_h,
                        (8.0 * scale).round() as i32,
                        (12.0 * scale).round() as i32,
                    );
                    assert_inside_work_area(
                        &format!("{name}_{bar_pos:?}_popover@{scale}"),
                        work,
                        px_x,
                        px_y,
                        pop_w,
                        pop_h,
                        (12.0 * scale).round() as i32,
                    );
                }
            }
        }
    }
}
