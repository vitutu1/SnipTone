use crate::frame::CapturedFrame;
use crate::ui::store::{
    AppStore, PREVIEW_MAX_DIM, WidgetId, close_snip_window, open_widget, show_all_from_tray,
};
use crate::ui::theme::*;
use gpui::prelude::FluentBuilder;
use gpui::*;
use image::{Frame, RgbaImage};
use smallvec::smallvec;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

#[derive(Debug, Clone, Copy)]
struct DragRect {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
}

impl DragRect {
    fn normalized(&self) -> (f32, f32, f32, f32) {
        (
            self.x1.min(self.x2),
            self.y1.min(self.y2),
            self.x1.max(self.x2),
            self.y1.max(self.y2),
        )
    }
}

/// On-demand fullscreen Region Snip window created only for the duration of a region drag on the
/// exact target monitor (supports both landscape 2560x1440 and portrait 1080x1920 monitors).
pub struct RegionSnipView {
    store: Entity<AppStore>,
    captured_full: Arc<CapturedFrame>,
    frozen_image: Option<Arc<RenderImage>>,
    rect: Option<DragRect>,
    dragging: bool,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    focus_handle: FocusHandle,
}

impl RegionSnipView {
    pub fn new(
        store: Entity<AppStore>,
        captured_full: Arc<CapturedFrame>,
        frozen_bgra: Vec<u8>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let frozen_image =
            RgbaImage::from_raw(captured_full.width, captured_full.height, frozen_bgra)
                .map(|buf| Arc::new(RenderImage::new(smallvec![Frame::new(buf)])));

        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);

        Self {
            store,
            captured_full,
            frozen_image,
            rect: None,
            dragging: false,
            bounds: Rc::new(Cell::new(Bounds::default())),
            focus_handle,
        }
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(img) = self.frozen_image.take() {
            let _ = window.drop_image(img);
        }
        let store = self.store.clone();
        cx.defer(move |cx| {
            close_snip_window(&store, cx);
            show_all_from_tray(&store, cx);
        });
    }

    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dragging = false;
        let Some(r) = self.rect.take() else {
            return;
        };
        let b = self.bounds.get();
        let bw = px_f32(b.size.width).max(1.0);
        let bh = px_f32(b.size.height).max(1.0);
        let ox = px_f32(b.origin.x);
        let oy = px_f32(b.origin.y);

        let (x0, y0, x1, y1) = r.normalized();
        if (x1 - x0) < 12.0 || (y1 - y0) < 12.0 {
            self.cancel(window, cx);
            return;
        }

        let u0 = ((x0 - ox) / bw).clamp(0.0, 1.0);
        let v0 = ((y0 - oy) / bh).clamp(0.0, 1.0);
        let u1 = ((x1 - ox) / bw).clamp(0.0, 1.0);
        let v1 = ((y1 - oy) / bh).clamp(0.0, 1.0);

        if let Some(img) = self.frozen_image.take() {
            let _ = window.drop_image(img);
        }

        let cropped = self.captured_full.crop_normalized(u0, v0, u1, v1);
        let store = self.store.clone();
        cx.defer(move |cx| {
            close_snip_window(&store, cx);
            if let Some(frame) = cropped {
                let preview = frame.downscale_for_preview(PREVIEW_MAX_DIM);
                store.update(cx, |s, cx| {
                    s.set_captured_frame(Arc::new(frame), Arc::new(preview));
                    s.apply_post_capture_actions();
                    cx.notify();
                });
                show_all_from_tray(&store, cx);
                open_widget(&store, WidgetId::Preview, cx);
            } else {
                show_all_from_tray(&store, cx);
            }
        });
    }
}

impl Render for RegionSnipView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bounds_cell = Rc::clone(&self.bounds);
        let rect_opt = self.rect;
        let fw = self.captured_full.width as f32;
        let fh = self.captured_full.height as f32;
        let b = self.bounds.get();
        let bw = px_f32(b.size.width).max(1.0);
        let bh = px_f32(b.size.height).max(1.0);

        div()
            .id("region-snip-root")
            .track_focus(&self.focus_handle)
            .relative()
            .size_full()
            .cursor_crosshair()
            .overflow_hidden()
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                if ev.keystroke.key == "escape" {
                    this.cancel(window, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _, window, cx| {
                    this.cancel(window, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    let x = px_f32(ev.position.x);
                    let y = px_f32(ev.position.y);
                    this.dragging = true;
                    this.rect = Some(DragRect {
                        x1: x,
                        y1: y,
                        x2: x,
                        y2: y,
                    });
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                if this.dragging {
                    if let Some(r) = &mut this.rect {
                        r.x2 = px_f32(ev.position.x);
                        r.y2 = px_f32(ev.position.y);
                        cx.notify();
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    if this.dragging {
                        this.commit(window, cx);
                    }
                }),
            )
            .child(
                canvas(
                    move |bounds, _, _| {
                        bounds_cell.set(bounds);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .when_some(self.frozen_image.clone(), |d, image| {
                d.child(img(image).size_full().object_fit(ObjectFit::Fill))
            })
            // Subtle dim scrim over frozen capture
            .child(div().absolute().inset_0().bg(rgba(0x00000058)))
            // Hint pill at top center
            .child(
                div()
                    .absolute()
                    .top(px(18.0))
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .px(px(14.0))
                            .py(px(7.0))
                            .rounded(px(10.0))
                            .bg(rgb(BAR_OUTER))
                            .border_1()
                            .border_color(bar_border())
                            .text_xs()
                            .text_color(rgb(TEXT))
                            .child("Drag to capture region · Esc or Right-click to cancel"),
                    ),
            )
            // Live selection rectangle
            .when_some(rect_opt, |d, r| {
                let (x0, y0, x1, y1) = r.normalized();
                let rw = (x1 - x0).max(1.0);
                let rh = (y1 - y0).max(1.0);
                let phys_w = ((rw / bw) * fw).round();
                let phys_h = ((rh / bh) * fh).round();
                d.child(
                    div()
                        .absolute()
                        .left(px(x0))
                        .top(px(y0))
                        .w(px(rw))
                        .h(px(rh))
                        .border_2()
                        .border_color(rgb(0xffffff))
                        .bg(rgba(0xffffff10))
                        .child(
                            div()
                                .absolute()
                                .top(px(-28.0))
                                .left_0()
                                .px(px(8.0))
                                .py(px(3.0))
                                .rounded(px(6.0))
                                .bg(rgb(BAR_OUTER))
                                .border_1()
                                .border_color(bar_border())
                                .text_size(px(11.0))
                                .text_color(rgb(TEXT))
                                .child(format!("{:.0} × {:.0}", phys_w, phys_h)),
                        ),
                )
            })
    }
}
