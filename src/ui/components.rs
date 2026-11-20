use crate::frame::{HIST_LOG_MAX, HIST_LOG_MIN, HISTOGRAM_BINS};
use crate::tonemap::{TonemapParams, sample_tone_curve};
use crate::ui::theme::*;
use gpui::prelude::FluentBuilder;
use gpui::*;
use std::cell::Cell;
use std::rc::Rc;

pub fn svg_icon(path: &'static str, size_px: f32, color_hex_val: u32) -> Svg {
    svg()
        .path(path)
        .size(px(size_px))
        .text_color(rgb(color_hex_val))
}

/// Game Bar tactile button (active/primary = #d7dbe2 with dark #15171b label/icon).
pub fn gb_btn(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    icon_path: Option<&'static str>,
    active: bool,
) -> Stateful<Div> {
    let (bg, hover_bg, fg) = if active {
        (ACTIVE_BG, ACTIVE_BG_HOVER, ACTIVE_FG)
    } else {
        (SURFACE_2, SURFACE_3, TEXT)
    };

    let mut el = div()
        .id(id)
        .h(px(32.0))
        .px_3()
        .rounded(px(10.0))
        .bg(rgb(bg))
        .text_xs()
        .font_weight(if active {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::MEDIUM
        })
        .text_color(rgb(fg))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.0))
        .cursor_pointer()
        .hover(move |s| s.bg(rgb(hover_bg)));

    if let Some(p) = icon_path {
        el = el.child(svg_icon(p, 13.0, fg));
    }
    el.child(label.into())
}

/// Monochrome Game Bar toggle switch pill.
pub fn toggle_pill(id: impl Into<ElementId>, on: bool) -> Stateful<Div> {
    div()
        .id(id)
        .w(px(38.0))
        .h(px(22.0))
        .rounded_full()
        .bg(rgb(if on { ACTIVE_BG } else { SURFACE_3 }))
        .relative()
        .cursor_pointer()
        .child(
            div()
                .absolute()
                .top(px(3.0))
                .left(px(if on { 19.0 } else { 3.0 }))
                .w(px(16.0))
                .h(px(16.0))
                .rounded_full()
                .bg(rgb(if on { ACTIVE_FG } else { 0xffffff })),
        )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliderId {
    SdrWhite,
    Exposure,
    PeakLum,
    Shoulder,
    Desat,
}

#[derive(Debug, Clone, Copy)]
pub struct SliderSpec {
    pub id: SliderId,
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
    pub step: f32,
    pub default_value: f32,
    pub value: f32,
    pub dimmed: bool,
}

impl SliderSpec {
    pub fn snap(&self, raw: f32) -> f32 {
        let clamped = raw.clamp(self.min, self.max);
        let snapped = (clamped / self.step).round() * self.step;
        snapped.clamp(self.min, self.max)
    }

    pub fn value_from_x(&self, x: Pixels, bounds: Bounds<Pixels>) -> f32 {
        let width = px_f32(bounds.size.width).max(1.0);
        let ratio = ((px_f32(x) - px_f32(bounds.origin.x)) / width).clamp(0.0, 1.0);
        self.snap(self.min + ratio * (self.max - self.min))
    }

    pub fn ratio(&self) -> f32 {
        ((self.value - self.min) / (self.max - self.min).max(1e-5)).clamp(0.0, 1.0)
    }

    pub fn default_ratio(&self) -> f32 {
        ((self.default_value - self.min) / (self.max - self.min).max(1e-5)).clamp(0.0, 1.0)
    }
}

/// Custom tactile monochrome Game Bar slider with real-time mouse drag, default detent,
/// scroll-wheel precision nudge, and click-value reset.
pub fn gamebar_slider<V: 'static>(
    spec: SliderSpec,
    value_text: String,
    bounds_cell: Rc<Cell<Bounds<Pixels>>>,
    dragging: bool,
    on_start_drag: impl Fn(&mut V, SliderId, f32, &mut Window, &mut Context<V>) + 'static,
    on_reset: impl Fn(&mut V, SliderId, &mut Window, &mut Context<V>) + 'static,
    on_nudge: impl Fn(&mut V, SliderId, f32, &mut Window, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> Div {
    let pct = spec.ratio();
    let def_pct = spec.default_ratio();
    let bounds_for_prepaint = Rc::clone(&bounds_cell);
    let bounds_for_down = Rc::clone(&bounds_cell);
    let slider_id = spec.id;

    let label_col = if spec.dimmed { DIM } else { MUTED };
    let val_col = if spec.dimmed { DIM } else { TEXT };

    div()
        .flex()
        .flex_col()
        .gap(px(5.0))
        .when(spec.dimmed, |d| d.opacity(0.42))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .text_xs()
                .child(div().text_color(rgb(label_col)).child(spec.label))
                .child(
                    div()
                        .id(SharedString::from(format!("slider-val-{:?}", slider_id)))
                        .px(px(7.0))
                        .py(px(2.0))
                        .rounded(px(6.0))
                        .bg(rgb(RECESS))
                        .border_1()
                        .border_color(subtle_border())
                        .text_color(rgb(val_col))
                        .cursor_pointer()
                        .hover(|s| s.border_color(rgba(0xffffff33)))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            on_reset(this, slider_id, window, cx);
                        }))
                        .child(value_text),
                ),
        )
        .child(
            div()
                .id(SharedString::from(format!("slider-track-{:?}", slider_id)))
                .relative()
                .w_full()
                .h(px(22.0))
                .flex()
                .items_center()
                .cursor_ew_resize()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                        let b = bounds_for_down.get();
                        let next = spec.value_from_x(ev.position.x, b);
                        on_start_drag(this, slider_id, next, window, cx);
                    }),
                )
                .on_scroll_wheel(cx.listener(move |this, ev: &ScrollWheelEvent, window, cx| {
                    let dy = match ev.delta {
                        ScrollDelta::Lines(p) => p.y,
                        ScrollDelta::Pixels(p) => px_f32(p.y) / 24.0,
                    };
                    if dy.abs() > 0.01 {
                        let dir = if dy > 0.0 { 1.0 } else { -1.0 };
                        on_nudge(this, slider_id, dir * spec.step, window, cx);
                    }
                }))
                .child(
                    // Invisible bounds recorder for exact 120 FPS pointer mapping
                    canvas(
                        move |bounds, _, _| {
                            bounds_for_prepaint.set(bounds);
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .inset_0(),
                )
                .child(
                    // Recessed track + bright monochrome fill
                    div()
                        .relative()
                        .w_full()
                        .h(px(6.0))
                        .rounded_full()
                        .bg(rgb(RECESS))
                        .overflow_hidden()
                        .child(div().h_full().w(relative(pct)).rounded_full().bg(rgb(ACTIVE_BG))),
                )
                .child(
                    // Default detent tick
                    div()
                        .absolute()
                        .top(px(5.0))
                        .left(relative(def_pct))
                        .ml(px(-1.0))
                        .w(px(2.0))
                        .h(px(12.0))
                        .rounded(px(1.0))
                        .bg(detent_color()),
                )
                .child(
                    // Squircle thumb
                    div()
                        .absolute()
                        .top(px(if dragging { 3.0 } else { 4.0 }))
                        .left(relative(pct))
                        .ml(px(if dragging { -8.0 } else { -7.0 }))
                        .w(px(if dragging { 16.0 } else { 14.0 }))
                        .h(px(if dragging { 16.0 } else { 14.0 }))
                        .rounded(px(5.0))
                        .bg(rgb(0xffffff))
                        .border_1()
                        .border_color(rgba(0x00000044)),
                ),
        )
}

/// GPU-drawn Tone Curve graph (Input 0..5x SDR White -> Output 0..1 sRGB) via GPUI PathBuilder.
pub fn tone_curve_view(params: TonemapParams) -> Div {
    let curve_pts = sample_tone_curve(&params, 5.0, 44);
    div()
        .w(px(112.0))
        .h(px(82.0))
        .rounded(px(10.0))
        .bg(rgb(RECESS))
        .border_1()
        .border_color(subtle_border())
        .overflow_hidden()
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let x0 = px_f32(bounds.origin.x) + 6.0;
                    let y0 = px_f32(bounds.origin.y) + 8.0;
                    let w = (px_f32(bounds.size.width) - 12.0).max(1.0);
                    let h = (px_f32(bounds.size.height) - 16.0).max(1.0);
                    let sdr_x = x0 + 0.2 * w; // 1.0x on 0..5x scale

                    // Amber vertical SDR White reference line
                    let mut sdr_line = PathBuilder::stroke(px(1.0));
                    sdr_line.move_to(point(px(sdr_x), px(y0)));
                    sdr_line.line_to(point(px(sdr_x), px(y0 + h)));
                    if let Ok(path) = sdr_line.build() {
                        window.paint_path(path, rgba(0xf59e0b8c));
                    }

                    // Faint Windows Clip reference polyline
                    let mut clip_ref = PathBuilder::stroke(px(1.0));
                    clip_ref.move_to(point(px(x0), px(y0 + h)));
                    clip_ref.line_to(point(px(sdr_x), px(y0)));
                    clip_ref.line_to(point(px(x0 + w), px(y0)));
                    if let Ok(path) = clip_ref.build() {
                        window.paint_path(path, rgba(0xffffff24));
                    }

                    // Active operator curve
                    let mut curve = PathBuilder::stroke(px(1.9));
                    for (idx, &(xn, yn)) in curve_pts.iter().enumerate() {
                        let pt = point(px(x0 + xn * w), px(y0 + (1.0 - yn) * h));
                        if idx == 0 {
                            curve.move_to(pt);
                        } else {
                            curve.line_to(pt);
                        }
                    }
                    if let Ok(path) = curve.build() {
                        window.paint_path(path, rgb(0xe4e7ee));
                    }
                },
            )
            .size_full(),
        )
}

/// 64-bin log-luminance histogram with live amber SDR White threshold marker.
pub fn histogram_view(histogram: &[f32; HISTOGRAM_BINS], sdr_white_nits: f32) -> Div {
    let sdr_pct = ((sdr_white_nits.max(0.1).log10() - HIST_LOG_MIN) / (HIST_LOG_MAX - HIST_LOG_MIN))
        .clamp(0.0, 1.0);

    let mut bars = div()
        .flex_1()
        .h(px(82.0))
        .rounded(px(10.0))
        .bg(rgb(RECESS))
        .border_1()
        .border_color(subtle_border())
        .relative()
        .px(px(6.0))
        .pt(px(10.0))
        .flex()
        .items_end()
        .gap(px(1.5))
        .overflow_hidden();

    for (i, &val) in histogram.iter().enumerate() {
        let bin_pct = i as f32 / (HISTOGRAM_BINS - 1) as f32;
        let is_hdr = bin_pct > sdr_pct;
        let h_px = (val * 68.0).max(2.0);
        bars = bars.child(
            div()
                .flex_1()
                .h(px(h_px))
                .rounded_t(px(1.5))
                .bg(if is_hdr { hist_hdr_bar() } else { hist_sdr_bar() }),
        );
    }

    bars.child(
        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(relative(sdr_pct))
            .w(px(1.5))
            .bg(rgb(AMBER)),
    )
}

pub fn stat_box(label: &'static str, value: String, amber: bool) -> Div {
    div()
        .flex_1()
        .px(px(8.0))
        .py(px(7.0))
        .rounded(px(9.0))
        .bg(rgb(RECESS))
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(div().text_size(px(10.0)).text_color(rgb(DIM)).child(label))
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(if amber { AMBER_BRIGHT } else { TEXT }))
                .child(value),
        )
}

/// Reusable monochrome Game Bar widget shell filling its own independent floating PopUp window.
pub fn widget_window_shell<V: 'static>(
    id_prefix: &'static str,
    title: &'static str,
    meta: Option<String>,
    pinned: bool,
    on_drag_start: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
    on_pin: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
    on_close: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
    body: Div,
    cx: &mut Context<V>,
) -> Div {
    div()
        .size_full()
        .rounded(px(16.0))
        .bg(rgb(WIDGET_BG))
        .border_1()
        .border_color(bar_border())
        .overflow_hidden()
        .flex()
        .flex_col()
        .child(
            div()
                .id(SharedString::from(format!("whead-{}", id_prefix)))
                .h(px(40.0))
                .pl(px(14.0))
                .pr(px(10.0))
                .rounded_t(px(15.0))
                .bg(rgb(WIDGET_HEAD))
                .border_b_1()
                .border_color(subtle_border())
                .flex()
                .items_center()
                .justify_between()
                .cursor_grab()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| on_drag_start(this, window, cx)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(TEXT))
                                .child(title),
                        )
                        .when_some(meta, |d, m| {
                            d.child(div().text_size(px(11.0)).text_color(rgb(DIM)).child(m))
                        }),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(3.0))
                        .child(
                            div()
                                .id(SharedString::from(format!("wpin-{}", id_prefix)))
                                .w(px(26.0))
                                .h(px(26.0))
                                .rounded(px(8.0))
                                .bg(if pinned {
                                    rgb(ACTIVE_BG)
                                } else {
                                    rgba(0x00000000)
                                })
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .hover(move |s| {
                                    s.bg(if pinned {
                                        rgb(ACTIVE_BG_HOVER)
                                    } else {
                                        hover_overlay()
                                    })
                                })
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    on_pin(this, window, cx);
                                }))
                                .child(svg_icon(
                                    "icons/pin.svg",
                                    13.0,
                                    if pinned { ACTIVE_FG } else { MUTED },
                                )),
                        )
                        .child(
                            div()
                                .id(SharedString::from(format!("wclose-{}", id_prefix)))
                                .w(px(26.0))
                                .h(px(26.0))
                                .rounded(px(8.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .hover(|s| s.bg(hover_overlay()))
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    on_close(this, window, cx);
                                }))
                                .child(svg_icon("icons/close.svg", 13.0, MUTED)),
                        ),
                ),
        )
        .child(body.flex_1().p(px(13.0)).flex().flex_col().gap(px(11.0)))
}
