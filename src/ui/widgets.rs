use crate::config::BarPosition;
use crate::tonemap::{GamutMapping, TonemapOperator};
use crate::tray::{begin_native_window_drag, set_windows_startup_enabled};
use crate::ui::components::{
    SliderId, gamebar_slider, gb_btn, histogram_view, stat_box, svg_icon, toggle_pill,
    tone_curve_view, widget_window_shell,
};
use crate::ui::store::{
    AppStore, MarkupShape, MarkupTool, WidgetId, close_widget, hide_all_to_tray, quit_application,
    set_bar_position, toggle_widget_pin,
};
use crate::ui::theme::*;
use gpui::prelude::FluentBuilder;
use gpui::*;
use image::{Frame, RgbaImage};
use smallvec::smallvec;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

/// Independent floating PopUp window hosting a single SnipTone widget.
pub struct WidgetWindowView {
    store: Entity<AppStore>,
    id: WidgetId,
    hwnd: isize,
    _sub: Subscription,

    // Preview window local GPU texture caches (uploaded only when respective versions change).
    // Dragging the A/B Split Curtain uses pure GPU scissor clipping -> 0.00ms CPU, 0 bytes GPU upload!
    cached_active_ver: u64,
    cached_active_image: Option<Arc<RenderImage>>,
    cached_baseline_ver: u64,
    cached_baseline_image: Option<Arc<RenderImage>>,
    preview_bounds: Rc<Cell<Bounds<Pixels>>>,
    split_ratio: f32,
    dragging_split: bool,
    draft_shape: Option<MarkupShape>,

    // Tonemap slider dragging local state
    active_slider: Option<SliderId>,
    slider_bounds: [Rc<Cell<Bounds<Pixels>>>; 5],
}

impl WidgetWindowView {
    pub fn new(
        store: Entity<AppStore>,
        id: WidgetId,
        hwnd: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let sub = cx.observe(&store, |_, _, cx| cx.notify());
        let mut view = Self {
            store,
            id,
            hwnd,
            _sub: sub,
            cached_active_ver: u64::MAX,
            cached_active_image: None,
            cached_baseline_ver: u64::MAX,
            cached_baseline_image: None,
            preview_bounds: Rc::new(Cell::new(Bounds::default())),
            split_ratio: 0.55,
            dragging_split: false,
            draft_shape: None,
            active_slider: None,
            slider_bounds: std::array::from_fn(|_| Rc::new(Cell::new(Bounds::default()))),
        };
        if id == WidgetId::Preview {
            view.sync_preview_textures(window, cx);
        }
        view
    }

    /// Immediately releases cached GPU atlas textures (called synchronously when hiding to tray).
    pub fn release_gpu_textures(&mut self, window: &mut Window) {
        if let Some(old) = self.cached_active_image.take() {
            let _ = window.drop_image(old);
        }
        if let Some(old) = self.cached_baseline_image.take() {
            let _ = window.drop_image(old);
        }
        self.cached_active_ver = u64::MAX;
        self.cached_baseline_ver = u64::MAX;
    }

    fn sync_preview_textures(&mut self, window: &mut Window, cx: &Context<Self>) {
        // Release GPU atlas textures immediately when parked in System Tray (0 idle VRAM)
        if self
            .store
            .read(cx)
            .hidden_in_tray
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            self.release_gpu_textures(window);
            return;
        }

        let (a_ver, a_bgra, b_ver, b_bgra) = {
            let s = self.store.read(cx);
            (
                s.preview_active_version,
                Arc::clone(&s.preview_active_bgra),
                s.preview_baseline_version,
                Arc::clone(&s.preview_baseline_bgra),
            )
        };

        if self.cached_active_ver != a_ver || self.cached_active_image.is_none() {
            if let Some(old) = self.cached_active_image.take() {
                let _ = window.drop_image(old);
            }
            let (w, h, ref bytes) = *a_bgra;
            if let Some(buf) = RgbaImage::from_raw(w, h, bytes.clone()) {
                self.cached_active_image =
                    Some(Arc::new(RenderImage::new(smallvec![Frame::new(buf)])));
                self.cached_active_ver = a_ver;
            }
        }

        if self.cached_baseline_ver != b_ver || self.cached_baseline_image.is_none() {
            if let Some(old) = self.cached_baseline_image.take() {
                let _ = window.drop_image(old);
            }
            let (w, h, ref bytes) = *b_bgra;
            if let Some(buf) = RgbaImage::from_raw(w, h, bytes.clone()) {
                self.cached_baseline_image =
                    Some(Arc::new(RenderImage::new(smallvec![Frame::new(buf)])));
                self.cached_baseline_ver = b_ver;
            }
        }
    }

    fn slider_bounds_for(&self, id: SliderId) -> Bounds<Pixels> {
        let idx = match id {
            SliderId::SdrWhite => 0,
            SliderId::Exposure => 1,
            SliderId::PeakLum => 2,
            SliderId::Shoulder => 3,
            SliderId::Desat => 4,
        };
        self.slider_bounds[idx].get()
    }

    fn handle_mouse_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        let px_x = px_f32(ev.position.x);
        let px_y = px_f32(ev.position.y);

        if let Some(slider_id) = self.active_slider {
            let spec = self.store.read(cx).slider_spec(slider_id);
            let b = self.slider_bounds_for(slider_id);
            let next = spec.value_from_x(ev.position.x, b);
            self.store.update(cx, |s, cx| {
                if s.apply_slider_value(slider_id, next) {
                    cx.notify();
                }
            });
        }

        if self.dragging_split {
            let b = self.preview_bounds.get();
            let bw = px_f32(b.size.width);
            if bw > 1.0 {
                // Pure local state update: GPU hardware scissor clip moves at 120-240Hz
                // with 0.00ms CPU tonemap and 0 bytes GPU texture upload!
                self.split_ratio = ((px_x - px_f32(b.origin.x)) / bw).clamp(0.05, 0.95);
                cx.notify();
            }
        }

        if let Some(draft) = &mut self.draft_shape {
            let b = self.preview_bounds.get();
            let bw = px_f32(b.size.width);
            let bh = px_f32(b.size.height);
            if bw > 1.0 && bh > 1.0 {
                draft.u2 = ((px_x - px_f32(b.origin.x)) / bw).clamp(0.0, 1.0);
                draft.v2 = ((px_y - px_f32(b.origin.y)) / bh).clamp(0.0, 1.0);
                cx.notify();
            }
        }
    }

    fn handle_mouse_up(&mut self, cx: &mut Context<Self>) {
        let mut dirty = false;
        if self.active_slider.take().is_some() {
            dirty = true;
        }
        if self.dragging_split {
            self.dragging_split = false;
            dirty = true;
        }
        if let Some(draft) = self.draft_shape.take() {
            let dist = ((draft.u2 - draft.u1).powi(2) + (draft.v2 - draft.v1).powi(2)).sqrt();
            if dist > 0.015 {
                self.store.update(cx, |s, cx| {
                    s.shapes.push(draft);
                    cx.notify();
                });
            }
            dirty = true;
        }
        if dirty {
            cx.notify();
        }
    }
}

impl Render for WidgetWindowView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.id == WidgetId::Preview {
            self.sync_preview_textures(window, cx);
        }

        let content = match self.id {
            WidgetId::Preview => self.render_preview(cx),
            WidgetId::Tonemap => self.render_tonemap(cx),
            WidgetId::Scope => self.render_scope(cx),
            WidgetId::Markup => self.render_markup(cx),
            WidgetId::Settings => self.render_settings(cx),
        };

        div()
            .id(SharedString::from(format!("widget-win-{:?}", self.id)))
            .size_full()
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                this.handle_mouse_move(ev, cx);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.handle_mouse_up(cx);
                }),
            )
            .child(content)
    }
}

impl WidgetWindowView {
    fn shell(&self, title: &'static str, meta: Option<String>, body: Div, cx: &mut Context<Self>) -> Div {
        let id = self.id;
        let pinned = self.store.read(cx).is_widget_pinned(id);
        let id_str = match id {
            WidgetId::Preview => "preview",
            WidgetId::Tonemap => "tonemap",
            WidgetId::Scope => "scope",
            WidgetId::Markup => "markup",
            WidgetId::Settings => "settings",
        };

        widget_window_shell(
            id_str,
            title,
            meta,
            pinned,
            |this, _, _| begin_native_window_drag(this.hwnd),
            move |this, _, cx| toggle_widget_pin(&this.store, id, cx),
            move |this, _, cx| {
                let store = this.store.clone();
                cx.defer(move |cx| close_widget(&store, id, cx));
            },
            body,
            cx,
        )
    }

    fn render_preview(&self, cx: &mut Context<Self>) -> Div {
        let s = self.store.read(cx);
        let meta = format!(
            "{}×{} · {} · {:.1}ms",
            s.frame_full.width,
            s.frame_full.height,
            if s.frame_full.is_hdr { "FP16" } else { "SDR" },
            s.last_tonemap_ms
        );
        let left_tag = if s.params.show_hdr_heatmap {
            "Heatmap"
        } else {
            s.params.operator.short_label()
        };
        let split_on = s.split_on && s.frame_full.is_hdr;
        let split_ratio = self.split_ratio;
        let heatmap_on = s.params.show_hdr_heatmap;
        let markup_open = s.is_widget_open(WidgetId::Markup);
        let shapes = s.shapes.clone();
        let draft = self.draft_shape;
        let status = s.status_line.clone();
        let preview_h = if s.frame_full.height > s.frame_full.width {
            264.0
        } else {
            204.0
        };

        // Exact pixel width of the viewport for pixel-aligned GPU scissor A/B split
        let vp_bounds = self.preview_bounds.get();
        let vp_w = px_f32(vp_bounds.size.width).max(376.0);
        let vp_h = px_f32(vp_bounds.size.height).max(preview_h);
        let split_px = (vp_w * split_ratio).clamp(1.0, vp_w);

        let bounds_prepaint = Rc::clone(&self.preview_bounds);
        let bounds_down = Rc::clone(&self.preview_bounds);

        let active_img = self.cached_active_image.clone();
        let base_img = self.cached_baseline_image.clone();

        let body = div()
            .child(
                div()
                    .id("preview-viewport")
                    .relative()
                    .w_full()
                    .h(px(preview_h))
                    .rounded(px(10.0))
                    .bg(rgb(BG_STAGE))
                    .border_1()
                    .border_color(subtle_border())
                    .overflow_hidden()
                    .when(markup_open, |d| d.cursor_crosshair())
                    .when(!markup_open && split_on, |d| d.cursor_ew_resize())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                            let b = bounds_down.get();
                            let bw = px_f32(b.size.width);
                            let bh = px_f32(b.size.height);
                            if bw <= 1.0 || bh <= 1.0 {
                                return;
                            }
                            if !this.store.read(cx).is_widget_open(WidgetId::Markup) {
                                // When Markup is closed and A/B Split is active, dragging anywhere
                                // on the preview smoothly scrubs the GPU-clipped split curtain.
                                let st = this.store.read(cx);
                                if st.split_on && st.frame_full.is_hdr {
                                    this.dragging_split = true;
                                    this.split_ratio =
                                        ((px_f32(ev.position.x) - px_f32(b.origin.x)) / bw)
                                            .clamp(0.05, 0.95);
                                    cx.notify();
                                }
                                return;
                            }
                            let u = ((px_f32(ev.position.x) - px_f32(b.origin.x)) / bw)
                                .clamp(0.0, 1.0);
                            let v = ((px_f32(ev.position.y) - px_f32(b.origin.y)) / bh)
                                .clamp(0.0, 1.0);
                            let (tool, color) = {
                                let st = this.store.read(cx);
                                (st.markup_tool, st.markup_color)
                            };
                            this.draft_shape = Some(MarkupShape {
                                tool,
                                color,
                                u1: u,
                                v1: v,
                                u2: u,
                                v2: v,
                            });
                            cx.notify();
                        }),
                    )
                    // Base layer: Windows Clip (when split is on) or Active Tonemap (when split is off)
                    .when(split_on, |d| {
                        d.when_some(base_img, |el, image| {
                            el.child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .child(
                                        img(image)
                                            .size_full()
                                            .rounded(px(9.0))
                                            .object_fit(ObjectFit::Contain),
                                    ),
                            )
                        })
                        .when_some(active_img.clone(), |el, image| {
                            // Top layer clipped by GPU scissor rect at `split_px` — 240 FPS zero-upload!
                            el.child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .bottom_0()
                                    .left_0()
                                    .w(px(split_px))
                                    .overflow_hidden()
                                    .child(
                                        div().w(px(vp_w)).h(px(vp_h)).child(
                                            img(image)
                                                .size_full()
                                                .rounded(px(9.0))
                                                .object_fit(ObjectFit::Contain),
                                        ),
                                    ),
                            )
                        })
                    })
                    .when(!split_on, |d| {
                        d.when_some(active_img, |el, image| {
                            el.child(
                                img(image)
                                    .size_full()
                                    .rounded(px(9.0))
                                    .object_fit(ObjectFit::Contain),
                            )
                        })
                    })
                    .child(
                        canvas(
                            move |bounds, _, _| bounds_prepaint.set(bounds),
                            move |bounds, _, window, _| {
                                let ox = px_f32(bounds.origin.x);
                                let oy = px_f32(bounds.origin.y);
                                let bw = px_f32(bounds.size.width);
                                let bh = px_f32(bounds.size.height);

                                for sh in shapes.iter().copied().chain(draft.into_iter()) {
                                    let x1 = ox + sh.u1 * bw;
                                    let y1 = oy + sh.v1 * bh;
                                    let x2 = ox + sh.u2 * bw;
                                    let y2 = oy + sh.v2 * bh;

                                    match sh.tool {
                                        MarkupTool::Box => {
                                            let lx = x1.min(x2);
                                            let ty = y1.min(y2);
                                            let rx = x1.max(x2);
                                            let by = y1.max(y2);
                                            let mut p = PathBuilder::stroke(px(2.0));
                                            p.move_to(point(px(lx), px(ty)));
                                            p.line_to(point(px(rx), px(ty)));
                                            p.line_to(point(px(rx), px(by)));
                                            p.line_to(point(px(lx), px(by)));
                                            p.line_to(point(px(lx), px(ty)));
                                            if let Ok(path) = p.build() {
                                                window.paint_path(path, color_hex(sh.color));
                                            }
                                        }
                                        MarkupTool::Arrow => {
                                            let mut p = PathBuilder::stroke(px(2.2));
                                            p.move_to(point(px(x1), px(y1)));
                                            p.line_to(point(px(x2), px(y2)));
                                            let dx = x2 - x1;
                                            let dy = y2 - y1;
                                            let len = (dx * dx + dy * dy).sqrt().max(1.0);
                                            let ux = dx / len;
                                            let uy = dy / len;
                                            let head = 9.0;
                                            p.move_to(point(
                                                px(x2 - ux * head - uy * head * 0.55),
                                                px(y2 - uy * head + ux * head * 0.55),
                                            ));
                                            p.line_to(point(px(x2), px(y2)));
                                            p.line_to(point(
                                                px(x2 - ux * head + uy * head * 0.55),
                                                px(y2 - uy * head - ux * head * 0.55),
                                            ));
                                            if let Ok(path) = p.build() {
                                                window.paint_path(path, color_hex(sh.color));
                                            }
                                        }
                                        MarkupTool::Blur => {
                                            window.paint_quad(fill(
                                                Bounds::new(
                                                    point(px(x1.min(x2)), px(y1.min(y2))),
                                                    size(px((x2 - x1).abs()), px((y2 - y1).abs())),
                                                ),
                                                rgba(0x0d0e11eb),
                                            ));
                                        }
                                    }
                                }
                            },
                        )
                        .absolute()
                        .inset_0(),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(8.0))
                            .left(px(8.0))
                            .px(px(8.0))
                            .py(px(3.0))
                            .rounded(px(6.0))
                            .bg(rgba(0x15171bdb))
                            .border_1()
                            .border_color(bar_border())
                            .text_size(px(10.5))
                            .text_color(rgb(TEXT))
                            .child(left_tag),
                    )
                    .when(split_on, |d| {
                        d.child(
                            div()
                                .absolute()
                                .top(px(8.0))
                                .right(px(8.0))
                                .px(px(8.0))
                                .py(px(3.0))
                                .rounded(px(6.0))
                                .bg(rgba(0x15171bdb))
                                .border_1()
                                .border_color(bar_border())
                                .text_size(px(10.5))
                                .text_color(rgb(TEXT))
                                .child("Win Clip"),
                        )
                        .child(
                            div()
                                .id("split-curtain-handle")
                                .absolute()
                                .top_0()
                                .bottom_0()
                                .left(relative(split_ratio))
                                .ml(px(-10.0))
                                .w(px(20.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_ew_resize()
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.dragging_split = true;
                                    }),
                                )
                                .child(
                                    div()
                                        .absolute()
                                        .top_0()
                                        .bottom_0()
                                        .left(px(9.0))
                                        .w(px(2.0))
                                        .bg(rgb(0xffffff)),
                                )
                                .child(
                                    div()
                                        .w(px(24.0))
                                        .h(px(24.0))
                                        .rounded(px(8.0))
                                        .bg(rgb(ACTIVE_BG))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(svg_icon("icons/grip.svg", 11.0, ACTIVE_FG)),
                                ),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(
                        gb_btn("btn-copy", "Copy", Some("icons/copy.svg"), true)
                            .flex_1()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.store.update(cx, |s, cx| {
                                    s.copy_to_clipboard();
                                    cx.notify();
                                });
                            })),
                    )
                    .child(
                        gb_btn("btn-png", "PNG", Some("icons/save.svg"), false)
                            .flex_1()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.store.update(cx, |s, cx| {
                                    s.save_png();
                                    cx.notify();
                                });
                            })),
                    )
                    .child(
                        gb_btn("btn-exr", "EXR", Some("icons/save.svg"), false)
                            .flex_1()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.store.update(cx, |s, cx| {
                                    s.save_exr();
                                    cx.notify();
                                });
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(
                        gb_btn("btn-ab", "A/B Split", Some("icons/split.svg"), split_on)
                            .flex_1()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.store.update(cx, |s, cx| {
                                    if s.frame_full.is_hdr {
                                        s.split_on = !s.split_on;
                                        cx.notify();
                                    }
                                });
                            })),
                    )
                    .child(
                        gb_btn("btn-heat", "Heatmap", Some("icons/heat.svg"), heatmap_on)
                            .flex_1()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.store.update(cx, |s, cx| {
                                    if s.frame_full.is_hdr {
                                        s.params.show_hdr_heatmap = !s.params.show_hdr_heatmap;
                                        s.recompute_preview();
                                        cx.notify();
                                    }
                                });
                            })),
                    ),
            )
            .when_some(status, |d, msg| {
                d.child(
                    div()
                        .text_size(px(10.5))
                        .text_color(rgb(MUTED))
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(svg_icon("icons/check.svg", 11.0, GREEN_BRIGHT))
                        .child(msg),
                )
            });

        self.shell("Capture", Some(meta), body, cx)
    }

    fn render_tonemap(&self, cx: &mut Context<Self>) -> Div {
        let s = self.store.read(cx);
        let active_op = s.params.operator;
        let heatmap_on = s.params.show_hdr_heatmap;
        let soft_gamut = s.params.gamut_mapping == GamutMapping::SoftLuminance;

        let specs = [
            (
                s.slider_spec(SliderId::SdrWhite),
                format!("{:.0} nits", s.params.sdr_white_nits),
                Rc::clone(&self.slider_bounds[0]),
            ),
            (
                s.slider_spec(SliderId::Exposure),
                format!("{:+.2} EV", s.params.exposure_ev),
                Rc::clone(&self.slider_bounds[1]),
            ),
            (
                s.slider_spec(SliderId::PeakLum),
                format!("{:.0} nits", s.params.max_luminance_nits),
                Rc::clone(&self.slider_bounds[2]),
            ),
            (
                s.slider_spec(SliderId::Shoulder),
                format!("{:.2}", s.params.shoulder_start),
                Rc::clone(&self.slider_bounds[3]),
            ),
            (
                s.slider_spec(SliderId::Desat),
                format!("{:.0}%", s.params.highlight_desat * 100.0),
                Rc::clone(&self.slider_bounds[4]),
            ),
        ];

        let mut op_rows = div().flex().flex_col().gap(px(5.0));
        for pair in TonemapOperator::ALL.chunks(2) {
            let mut row = div().flex().gap(px(5.0));
            for &op in pair {
                let is_sel = op == active_op && !heatmap_on;
                row = row.child(
                    div()
                        .id(SharedString::from(format!("op-tile-{:?}", op)))
                        .flex_1()
                        .h(px(32.0))
                        .px(px(10.0))
                        .rounded(px(9.0))
                        .bg(rgb(if is_sel { ACTIVE_BG } else { SURFACE_2 }))
                        .flex()
                        .items_center()
                        .text_xs()
                        .font_weight(if is_sel {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::MEDIUM
                        })
                        .text_color(rgb(if is_sel { ACTIVE_FG } else { MUTED }))
                        .cursor_pointer()
                        .hover(move |st| {
                            st.bg(rgb(if is_sel { ACTIVE_BG_HOVER } else { SURFACE_3 }))
                                .text_color(rgb(if is_sel { ACTIVE_FG } else { TEXT }))
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.store.update(cx, |st, cx| {
                                st.params.operator = op;
                                st.params.show_hdr_heatmap = false;
                                st.recompute_preview();
                                cx.notify();
                            });
                        }))
                        .child(op.short_label()),
                );
            }
            if pair.len() == 1 {
                row = row.child(div().flex_1());
            }
            op_rows = op_rows.child(row);
        }

        let mut body = div().child(op_rows);
        for (spec, val_text, bounds_cell) in specs {
            let dragging = self.active_slider == Some(spec.id);
            body = body.child(gamebar_slider(
                spec,
                val_text,
                bounds_cell,
                dragging,
                |this: &mut Self, id, val, _, cx| {
                    this.active_slider = Some(id);
                    this.store.update(cx, |st, cx| {
                        st.apply_slider_value(id, val);
                        cx.notify();
                    });
                },
                |this: &mut Self, id, _, cx| {
                    this.store.update(cx, |st, cx| {
                        st.reset_single_slider(id);
                        cx.notify();
                    });
                },
                |this: &mut Self, id, delta, _, cx| {
                    this.store.update(cx, |st, cx| {
                        st.nudge_slider(id, delta);
                        cx.notify();
                    });
                },
                cx,
            ));
        }

        body = body
            .child(
                div()
                    .p(px(3.0))
                    .rounded(px(10.0))
                    .bg(rgb(RECESS))
                    .flex()
                    .gap(px(3.0))
                    .child(
                        div()
                            .id("gamut-soft")
                            .flex_1()
                            .h(px(26.0))
                            .rounded(px(7.0))
                            .bg(if soft_gamut {
                                rgb(ACTIVE_BG)
                            } else {
                                rgba(0x00000000)
                            })
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(11.5))
                            .font_weight(if soft_gamut {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::MEDIUM
                            })
                            .text_color(rgb(if soft_gamut { ACTIVE_FG } else { MUTED }))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.store.update(cx, |st, cx| {
                                    st.params.gamut_mapping = GamutMapping::SoftLuminance;
                                    st.recompute_preview();
                                    cx.notify();
                                });
                            }))
                            .child("Soft Gamut"),
                    )
                    .child(
                        div()
                            .id("gamut-clamp")
                            .flex_1()
                            .h(px(26.0))
                            .rounded(px(7.0))
                            .bg(if !soft_gamut {
                                rgb(ACTIVE_BG)
                            } else {
                                rgba(0x00000000)
                            })
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(11.5))
                            .font_weight(if !soft_gamut {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::MEDIUM
                            })
                            .text_color(rgb(if !soft_gamut { ACTIVE_FG } else { MUTED }))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.store.update(cx, |st, cx| {
                                    st.params.gamut_mapping = GamutMapping::HardClamp;
                                    st.recompute_preview();
                                    cx.notify();
                                });
                            }))
                            .child("Clamp"),
                    ),
            )
            .child(
                gb_btn(
                    "tonemap-reset-default",
                    "Reset to Default",
                    Some("icons/reset.svg"),
                    false,
                )
                .w_full()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.store.update(cx, |st, cx| {
                        st.reset_tonemap_defaults();
                        cx.notify();
                    });
                })),
            );

        self.shell("Tonemap", Some(active_op.short_label().to_string()), body, cx)
    }

    fn render_scope(&self, cx: &mut Context<Self>) -> Div {
        let s = self.store.read(cx);
        let stats = s.frame_full.stats.clone();
        let params = s.params;

        let body = div()
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(tone_curve_view(params))
                    .child(histogram_view(&stats.histogram, params.sdr_white_nits)),
            )
            .child(
                div()
                    .flex()
                    .gap(px(5.0))
                    .child(stat_box("P50", format!("{:.0}n", stats.p50_nits), false))
                    .child(stat_box("P99", format!("{:.0}n", stats.p99_nits), false))
                    .child(stat_box("Peak", format!("{:.0}n", stats.max_nits), true))
                    .child(stat_box(
                        "> SDR",
                        format!("{:.1}%", stats.pct_above_sdr_white),
                        false,
                    )),
            );

        self.shell("HDR Scope", Some("Log Nits".to_string()), body, cx)
    }

    fn render_markup(&self, cx: &mut Context<Self>) -> Div {
        let (tool, color, count) = {
            let s = self.store.read(cx);
            (s.markup_tool, s.markup_color, s.shapes.len())
        };
        let palette: [u32; 4] = [0xef4444, 0xf59e0b, 0xd7dbe2, 0x22c55e];

        let mut swatches = div().flex().items_center().gap(px(6.0));
        for col in palette {
            let active = color == col;
            swatches = swatches.child(
                div()
                    .id(SharedString::from(format!("sw-{:x}", col)))
                    .w(px(20.0))
                    .h(px(20.0))
                    .rounded(px(6.0))
                    .bg(rgb(col))
                    .border_2()
                    .border_color(if active {
                        rgb(0xffffff)
                    } else {
                        rgba(0x00000000)
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.store.update(cx, |s, cx| {
                            s.markup_color = col;
                            cx.notify();
                        });
                    })),
            );
        }

        let body = div()
            .child(
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(
                        gb_btn("mk-box", "Box", Some("icons/box.svg"), tool == MarkupTool::Box)
                            .flex_1()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.store.update(cx, |s, cx| {
                                    s.markup_tool = MarkupTool::Box;
                                    cx.notify();
                                });
                            })),
                    )
                    .child(
                        gb_btn(
                            "mk-arrow",
                            "Arrow",
                            Some("icons/arrow.svg"),
                            tool == MarkupTool::Arrow,
                        )
                        .flex_1()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.store.update(cx, |s, cx| {
                                s.markup_tool = MarkupTool::Arrow;
                                cx.notify();
                            });
                        })),
                    )
                    .child(
                        gb_btn(
                            "mk-blur",
                            "Blur",
                            Some("icons/redact.svg"),
                            tool == MarkupTool::Blur,
                        )
                        .flex_1()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.store.update(cx, |s, cx| {
                                s.markup_tool = MarkupTool::Blur;
                                cx.notify();
                            });
                        })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(swatches)
                    .child(
                        div()
                            .flex()
                            .gap(px(4.0))
                            .child(
                                gb_btn("mk-undo", "Undo", None, false)
                                    .h(px(26.0))
                                    .px(px(9.0))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.store.update(cx, |s, cx| {
                                            s.shapes.pop();
                                            cx.notify();
                                        });
                                    })),
                            )
                            .child(
                                gb_btn("mk-clear", "Clear", None, false)
                                    .h(px(26.0))
                                    .px(px(9.0))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.store.update(cx, |s, cx| {
                                            s.shapes.clear();
                                            cx.notify();
                                        });
                                    })),
                            ),
                    ),
            );

        self.shell("Markup", Some(format!("{}", count)), body, cx)
    }

    fn render_settings(&self, cx: &mut Context<Self>) -> Div {
        let cfg = self.store.read(cx).config.clone();
        let sc = &cfg.shortcuts;

        let shortcut_row = |id: &'static str, label: &'static str, key_str: String| {
            div()
                .id(id)
                .flex()
                .items_center()
                .justify_between()
                .text_xs()
                .cursor_pointer()
                .child(div().text_color(rgb(MUTED)).child(label))
                .child(
                    div()
                        .px(px(8.0))
                        .py(px(4.0))
                        .rounded(px(6.0))
                        .bg(rgb(RECESS))
                        .border_1()
                        .border_color(bar_border())
                        .text_size(px(11.0))
                        .text_color(rgb(TEXT))
                        .hover(|s| s.bg(rgb(SURFACE_3)))
                        .child(key_str),
                )
        };

        let body = div()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_xs()
                    .child(div().text_color(rgb(TEXT)).child("Auto-copy clipboard"))
                    .child(toggle_pill("set-autocopy", cfg.auto_copy).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.store.update(cx, |s, cx| {
                                s.config.auto_copy = !s.config.auto_copy;
                                s.config.save();
                                cx.notify();
                            });
                        },
                    ))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_xs()
                    .child(div().text_color(rgb(TEXT)).child("Auto-save PNG"))
                    .child(toggle_pill("set-autosave", cfg.auto_save_png).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.store.update(cx, |s, cx| {
                                s.config.auto_save_png = !s.config.auto_save_png;
                                s.config.save();
                                cx.notify();
                            });
                        },
                    ))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_xs()
                    .child(div().text_color(rgb(TEXT)).child("Capture cursor"))
                    .child(toggle_pill("set-cursor", cfg.capture_cursor).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.store.update(cx, |s, cx| {
                                s.config.capture_cursor = !s.config.capture_cursor;
                                s.config.save();
                                cx.notify();
                            });
                        },
                    ))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_xs()
                    .child(div().text_color(rgb(TEXT)).child("Start with Windows"))
                    .child(
                        toggle_pill("set-startup", cfg.start_with_windows).on_click(cx.listener(
                            |this, _, _, cx| {
                                let next = !this.store.read(cx).config.start_with_windows;
                                match set_windows_startup_enabled(next) {
                                    Ok(()) => {
                                        this.store.update(cx, |s, cx| {
                                            s.config.start_with_windows = next;
                                            s.config.save();
                                            s.status_line = Some(if next {
                                                "Enabled startup with Windows".to_string()
                                            } else {
                                                "Disabled startup with Windows".to_string()
                                            });
                                            cx.notify();
                                        });
                                    }
                                    Err(err) => {
                                        this.store.update(cx, |s, cx| {
                                            s.status_line = Some(err);
                                            cx.notify();
                                        });
                                    }
                                }
                            },
                        )),
                    ),
            )
            .child(div().h(px(1.0)).bg(subtle_border()))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(TEXT))
                                    .child("Game Bar position"),
                            )
                            .child(
                                div()
                                    .text_size(px(10.5))
                                    .text_color(rgb(DIM))
                                    .child(if cfg.bar_position.is_vertical() {
                                        "Vertical"
                                    } else {
                                        "Horizontal"
                                    }),
                            ),
                    )
                    .child({
                        let mut seg = div()
                            .p(px(3.0))
                            .rounded(px(10.0))
                            .bg(rgb(RECESS))
                            .flex()
                            .gap(px(3.0));
                        for &pos in BarPosition::ALL {
                            let active = cfg.bar_position == pos;
                            seg = seg.child(
                                div()
                                    .id(SharedString::from(format!("barpos-{:?}", pos)))
                                    .flex_1()
                                    .h(px(26.0))
                                    .rounded(px(7.0))
                                    .bg(if active {
                                        rgb(ACTIVE_BG)
                                    } else {
                                        rgba(0x00000000)
                                    })
                                    .text_size(px(11.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(if active { ACTIVE_FG } else { MUTED }))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover(move |s| {
                                        if active {
                                            s.bg(rgb(ACTIVE_BG_HOVER))
                                        } else {
                                            s.bg(hover_overlay()).text_color(rgb(TEXT))
                                        }
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let store = this.store.clone();
                                        cx.defer(move |cx| set_bar_position(&store, pos, cx));
                                    }))
                                    .child(pos.label()),
                            );
                        }
                        seg
                    }),
            )
            .child(div().h(px(1.0)).bg(subtle_border()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(svg_icon("icons/keyboard.svg", 13.0, MUTED))
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(TEXT))
                                    .child("Shortcuts"),
                            ),
                    )
                    .child(div().text_size(px(10.5)).text_color(rgb(DIM)).child("Click to cycle")),
            )
            .child(
                shortcut_row("hk-region", "Region Snip", sc.region_snip.display_label()).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.store.update(cx, |s, cx| {
                            s.config.cycle_region_shortcut();
                            s.sync_hotkeys();
                            cx.notify();
                        });
                    }),
                ),
            )
            .child(
                shortcut_row("hk-full", "Full Screen", sc.full_screen.display_label()).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.store.update(cx, |s, cx| {
                            s.config.cycle_fullscreen_shortcut();
                            s.sync_hotkeys();
                            cx.notify();
                        });
                    }),
                ),
            )
            .child(
                shortcut_row("hk-hud", "Show HUD", sc.toggle_hud.display_label()).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.store.update(cx, |s, cx| {
                            s.config.cycle_hud_shortcut();
                            s.sync_hotkeys();
                            cx.notify();
                        });
                    }),
                ),
            )
            .child(div().h(px(1.0)).bg(subtle_border()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_size(px(11.0))
                    .child(div().text_color(rgb(MUTED)).child("Output"))
                    .child(div().text_color(rgb(ACTIVE_BG)).child("Pictures\\SnipTone")),
            )
            .child(
                div()
                    .flex()
                    .gap(px(6.0))
                    .pt(px(2.0))
                    .child(
                        gb_btn("btn-hide-tray", "Hide to Tray", None, false)
                            .flex_1()
                            .on_click(cx.listener(|this, _, _, cx| {
                                let store = this.store.clone();
                                cx.defer(move |cx| hide_all_to_tray(&store, cx));
                            })),
                    )
                    .child(
                        gb_btn("btn-quit-app", "Quit", None, false)
                            .flex_1()
                            .on_click(cx.listener(|this, _, _, cx| {
                                quit_application(&this.store, cx);
                            })),
                    ),
            );

        self.shell("Settings", None, body, cx)
    }
}
