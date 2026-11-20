use crate::tray::begin_native_window_drag;
use crate::ui::components::svg_icon;
use crate::ui::store::{
    AppStore, PopoverKind, WidgetId, close_popover, hide_all_to_tray, hide_floating_tooltip,
    move_gamebar_to_display, open_widget, show_floating_tooltip, toggle_popover, toggle_widget,
    trigger_capture,
};
use crate::ui::theme::*;
use gpui::prelude::FluentBuilder;
use gpui::*;
use std::time::Duration;

pub struct GameBarView {
    store: Entity<AppStore>,
    hwnd: isize,
    focus_handle: FocusHandle,
    _sub: Subscription,
    hovered_tip: Option<&'static str>,
    tip_task: Option<Task<()>>,
}

impl GameBarView {
    pub fn new(
        store: Entity<AppStore>,
        hwnd: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let sub = cx.observe(&store, |_, _, cx| cx.notify());
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);
        Self {
            store,
            hwnd,
            focus_handle,
            _sub: sub,
            hovered_tip: None,
            tip_task: None,
        }
    }

    pub fn clear_hover_tip(&mut self, cx: &mut Context<Self>) {
        self.tip_task = None;
        self.hovered_tip = None;
        hide_floating_tooltip(&self.store, cx);
    }

    fn set_hover_tip(
        &mut self,
        hovered: bool,
        text: &'static str,
        center_x: f32,
        scale: f32,
        cx: &mut Context<Self>,
    ) {
        if !hovered {
            if self.hovered_tip == Some(text) {
                self.hovered_tip = None;
                let store = self.store.clone();
                // 45ms grace window so sliding between adjacent buttons reuses the floating tooltip seamlessly
                self.tip_task = Some(cx.spawn(async move |_this, cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(45))
                        .await;
                    let _ = cx.update(|cx| {
                        hide_floating_tooltip(&store, cx);
                    });
                }));
            }
            return;
        }

        self.hovered_tip = Some(text);
        let already_open = self.store.read(cx).tooltip_window.is_some();

        // If the floating tooltip is already visible, reposition & update text immediately;
        // otherwise wait 180ms of hover settle so passing the mouse across doesn't flash.
        if already_open {
            self.tip_task = None;
            show_floating_tooltip(&self.store, text, center_x, scale, cx);
        } else {
            let store = self.store.clone();
            self.tip_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(180))
                    .await;
                let _ = this.update(cx, |view, cx| {
                    if view.hovered_tip == Some(text) {
                        show_floating_tooltip(&store, text, center_x, scale, cx);
                    }
                });
            }));
        }
    }
}

fn sq_icon_btn(
    id: &'static str,
    icon_path: &'static str,
    tip_text: &'static str,
    tip_x: f32,
    active: bool,
    on_click: impl Fn(&mut GameBarView, &mut Window, &mut Context<GameBarView>) + 'static,
    cx: &mut Context<GameBarView>,
) -> Stateful<Div> {
    let icon_col = if active { ACTIVE_FG } else { ICON_LIGHT };
    div()
        .id(id)
        .flex_shrink_0()
        .w(px(40.0))
        .h(px(40.0))
        .rounded(px(12.0))
        .bg(if active {
            rgb(ACTIVE_BG)
        } else {
            rgba(0x00000000)
        })
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(move |s| {
            s.bg(if active {
                rgb(ACTIVE_BG_HOVER)
            } else {
                hover_overlay()
            })
        })
        .on_hover(cx.listener(move |this, &hovered, window, cx| {
            this.set_hover_tip(hovered, tip_text, tip_x, window.scale_factor(), cx);
        }))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |this, _, window, cx| {
            this.clear_hover_tip(cx);
            on_click(this, window, cx);
        }))
        .child(svg_icon(icon_path, 19.0, icon_col))
}

impl Render for GameBarView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let s = self.store.read(cx);
        let selected = s.displays.get(s.selected_display);
        let is_hdr = selected.map(|d| d.is_hdr).unwrap_or(s.frame_full.is_hdr);
        let sdr_nits = selected
            .map(|d| d.sdr_white_nits)
            .unwrap_or(s.params.sdr_white_nits);

        let readout_text = if s.show_nits_readout {
            if is_hdr {
                format!("{:.0} nits", s.params.sdr_white_nits)
            } else {
                "SDR".to_string()
            }
        } else {
            s.clock_str.clone()
        };

        let vertical = s.config.bar_position.is_vertical();
        let delay_secs = s.config.default_delay_secs;
        let is_capturing = s.is_capturing;
        let snip_open = s.snip_window.is_some();
        let preview_open = s.is_widget_open(WidgetId::Preview);
        let tonemap_open = s.is_widget_open(WidgetId::Tonemap);
        let scope_open = s.is_widget_open(WidgetId::Scope);
        let markup_open = s.is_widget_open(WidgetId::Markup);

        // Compact 2-line readout for the 58px-wide vertical bar
        let (vert_top, vert_bot) = if s.show_nits_readout {
            if is_hdr {
                (format!("{:.0}", s.params.sdr_white_nits), "nits".to_string())
            } else {
                ("SDR".to_string(), "8b".to_string())
            }
        } else {
            let mut parts = s.clock_str.split_whitespace();
            let t = parts.next().unwrap_or("").to_string();
            let ampm = parts.next().unwrap_or("").to_string();
            (t, ampm)
        };

        let disp_tip = if vertical { 39.0 } else { 44.0 };
        let home_tip = if vertical { 98.5 } else { 104.0 };
        let region_tip = if vertical { 167.0 } else { 174.0 };
        let full_tip = if vertical { 215.0 } else { 222.0 };
        let preview_tip = if vertical { 263.0 } else { 270.0 };
        let tonemap_tip = if vertical { 311.0 } else { 318.0 };
        let scope_tip = if vertical { 359.0 } else { 366.0 };
        let markup_tip = if vertical { 407.0 } else { 414.0 };
        let clock_tip = if vertical { 472.0 } else { 522.0 };
        let delay_tip = if vertical { 513.0 } else { 585.0 };
        let close_tip = if vertical { 564.0 } else { 636.0 };

        let zone_start = div()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(8.0))
            .when(vertical, |d| d.w_full().flex_col().pt(px(10.0)).pb(px(12.0)))
            .when(!vertical, |d| d.h_full().pl(px(10.0)).pr(px(12.0)))
            .child(
                div()
                    .id("gb-display-trigger")
                    .rounded(px(12.0))
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgba(0xffffff0f)))
                    .when(vertical, |d| d.flex_col().p(px(3.0)).gap(px(2.0)))
                    .when(!vertical, |d| d.p(px(4.0)).gap(px(6.0)))
                    .on_hover(cx.listener(move |this, &hovered, window, cx| {
                        this.set_hover_tip(
                            hovered,
                            "Switch display",
                            disp_tip,
                            window.scale_factor(),
                            cx,
                        );
                    }))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.clear_hover_tip(cx);
                        toggle_popover(&this.store, PopoverKind::Display, cx);
                    }))
                    .child(
                        div()
                            .w(px(38.0))
                            .h(px(38.0))
                            .rounded(px(10.0))
                            .bg(rgb(0x272b34))
                            .border_1()
                            .border_color(rgba(0xffffff24))
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .text_size(px(9.5))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(rgb(TEXT))
                                    .child(if is_hdr { "HDR" } else { "SDR" }),
                            )
                            .child(
                                div()
                                    .text_size(px(8.5))
                                    .text_color(rgb(MUTED))
                                    .child(if is_hdr {
                                        format!("{:.0}n", sdr_nits)
                                    } else {
                                        "8b".to_string()
                                    }),
                            ),
                    )
                    .child(svg_icon("icons/chev.svg", 10.0, MUTED)),
            )
            .child(
                div()
                    .p(px(2.5))
                    .rounded(px(15.0))
                    .border_2()
                    .border_color(rgb(0xffffff))
                    .child(
                        div()
                            .id("gb-home-menu")
                            .relative()
                            .w(px(38.0))
                            .h(px(38.0))
                            .rounded(px(11.0))
                            .bg(rgb(HOME_BG))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(HOME_HOVER)))
                            .on_hover(cx.listener(move |this, &hovered, window, cx| {
                                this.set_hover_tip(
                                    hovered,
                                    "Capture menu & shortcuts",
                                    home_tip,
                                    window.scale_factor(),
                                    cx,
                                );
                            }))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.clear_hover_tip(cx);
                                toggle_popover(&this.store, PopoverKind::HomeMenu, cx);
                            }))
                            .when(is_hdr, |d| {
                                d.child(
                                    div()
                                        .absolute()
                                        .top(px(4.0))
                                        .left(px(4.0))
                                        .w(px(9.0))
                                        .h(px(9.0))
                                        .rounded_full()
                                        .bg(rgb(GREEN))
                                        .border_2()
                                        .border_color(rgb(HOME_BG)),
                                )
                            })
                            .child(svg_icon("icons/grid.svg", 18.0, 0xf3f5f8)),
                    ),
            );

        let zone_center = div()
            .flex_shrink_0()
            .bg(rgb(BAR_CENTER))
            .border_color(rgba(0xffffff0a))
            .flex()
            .items_center()
            .gap(px(8.0))
            .when(vertical, |d| {
                d.w_full().flex_col().py(px(12.0)).border_t_1().border_b_1()
            })
            .when(!vertical, |d| {
                d.h_full().px(px(12.0)).border_l_1().border_r_1()
            })
            .child(sq_icon_btn(
                "gb-region",
                "icons/region.svg",
                "Snip region",
                region_tip,
                snip_open,
                |this, _, cx| {
                    let delay = this.store.read(cx).config.default_delay_secs;
                    trigger_capture(&this.store, delay, true, cx);
                },
                cx,
            ))
            .child(sq_icon_btn(
                "gb-full",
                "icons/camera.svg",
                "Print full screen",
                full_tip,
                is_capturing,
                |this, _, cx| {
                    let delay = this.store.read(cx).config.default_delay_secs;
                    trigger_capture(&this.store, delay, false, cx);
                },
                cx,
            ))
            .child(sq_icon_btn(
                "gb-preview",
                "icons/preview.svg",
                "Capture preview",
                preview_tip,
                preview_open,
                |this, _, cx| {
                    toggle_widget(&this.store, WidgetId::Preview, cx);
                },
                cx,
            ))
            .child(sq_icon_btn(
                "gb-tonemap",
                "icons/sliders.svg",
                "Tonemap controls",
                tonemap_tip,
                tonemap_open,
                |this, _, cx| {
                    toggle_widget(&this.store, WidgetId::Tonemap, cx);
                },
                cx,
            ))
            .child(sq_icon_btn(
                "gb-scope",
                "icons/scope.svg",
                "HDR scope",
                scope_tip,
                scope_open,
                |this, _, cx| {
                    toggle_widget(&this.store, WidgetId::Scope, cx);
                },
                cx,
            ))
            .child(sq_icon_btn(
                "gb-markup",
                "icons/markup.svg",
                "Markup tools",
                markup_tip,
                markup_open,
                |this, _, cx| {
                    toggle_widget(&this.store, WidgetId::Markup, cx);
                },
                cx,
            ));

        let clock_el = div()
            .id("gb-clock")
            .flex_shrink_0()
            .rounded(px(8.0))
            .cursor_pointer()
            .hover(|s| s.bg(rgba(0xffffff12)))
            .on_hover(cx.listener(move |this, &hovered, window, cx| {
                this.set_hover_tip(
                    hovered,
                    "Toggle clock / SDR nits",
                    clock_tip,
                    window.scale_factor(),
                    cx,
                );
            }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, _, cx| {
                this.store.update(cx, |s, cx| {
                    s.show_nits_readout = !s.show_nits_readout;
                    cx.notify();
                });
            }))
            .when(vertical, |d| {
                d.w(px(46.0))
                    .py(px(4.0))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .text_size(px(10.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(0xe4e7ee))
                            .child(vert_top),
                    )
                    .child(
                        div()
                            .text_size(px(8.5))
                            .text_color(rgb(MUTED))
                            .child(vert_bot),
                    )
            })
            .when(!vertical, |d| {
                d.px(px(6.0))
                    .py(px(6.0))
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(0xe4e7ee))
                    .child(readout_text)
            });

        let divider = if vertical {
            div().w(px(20.0)).h(px(1.0)).bg(rgba(0xffffff18))
        } else {
            div().w(px(1.0)).h(px(20.0)).bg(rgba(0xffffff18))
        };

        let zone_end = div()
            .flex_1()
            .flex()
            .items_center()
            .justify_end()
            .gap(px(8.0))
            .when(vertical, |d| d.w_full().flex_col().pt(px(12.0)).pb(px(12.0)))
            .when(!vertical, |d| d.h_full().pl(px(14.0)).pr(px(12.0)))
            .child(clock_el)
            .child(
                div()
                    .id("gb-delay")
                    .flex_shrink_0()
                    .relative()
                    .w(px(34.0))
                    .h(px(34.0))
                    .rounded(px(9.0))
                    .bg(if delay_secs > 0 {
                        rgb(ACTIVE_BG)
                    } else {
                        rgba(0x00000000)
                    })
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(move |s| {
                        s.bg(if delay_secs > 0 {
                            rgb(ACTIVE_BG_HOVER)
                        } else {
                            hover_overlay()
                        })
                    })
                    .on_hover(cx.listener(move |this, &hovered, window, cx| {
                        this.set_hover_tip(
                            hovered,
                            "Capture delay",
                            delay_tip,
                            window.scale_factor(),
                            cx,
                        );
                    }))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.store.update(cx, |s, cx| {
                            s.config.default_delay_secs = match s.config.default_delay_secs {
                                0 => 2,
                                2 => 3,
                                3 => 5,
                                _ => 0,
                            };
                            s.config.save();
                            cx.notify();
                        });
                    }))
                    .child(svg_icon(
                        "icons/timer.svg",
                        17.0,
                        if delay_secs > 0 { ACTIVE_FG } else { 0xd6dae2 },
                    ))
                    .when(delay_secs > 0, |d| {
                        d.child(
                            div()
                                .absolute()
                                .top(px(1.0))
                                .right(px(1.0))
                                .px(px(3.0))
                                .rounded(px(4.0))
                                .bg(rgb(BAR_OUTER))
                                .text_size(px(8.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(TEXT))
                                .child(format!("{}s", delay_secs)),
                        )
                    }),
            )
            .child(divider)
            .child(
                div()
                    .id("gb-hide-tray")
                    .flex_shrink_0()
                    .w(px(34.0))
                    .h(px(34.0))
                    .rounded(px(9.0))
                    .bg(rgb(SURFACE_2))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(0xb91c1c)))
                    .on_hover(cx.listener(move |this, &hovered, window, cx| {
                        this.set_hover_tip(
                            hovered,
                            "Close to tray",
                            close_tip,
                            window.scale_factor(),
                            cx,
                        );
                    }))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.clear_hover_tip(cx);
                        hide_all_to_tray(&this.store, cx);
                    }))
                    .child(svg_icon("icons/close.svg", 14.0, TEXT)),
            );

        div()
            .id("gamebar-capsule")
            .track_focus(&self.focus_handle)
            .size_full()
            .rounded(px(16.0))
            .bg(rgb(BAR_OUTER))
            .border_1()
            .border_color(bar_border())
            .flex()
            .when(vertical, |d| d.flex_col())
            .overflow_hidden()
            .cursor_grab()
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                match ev.keystroke.key.as_str() {
                    "escape" => {
                        if this.store.read(cx).popover_window.is_some() {
                            close_popover(&this.store, cx);
                        } else {
                            hide_all_to_tray(&this.store, cx);
                        }
                    }
                    "b" | "space" => {
                        this.store.update(cx, |s, cx| {
                            if s.frame_full.is_hdr {
                                s.split_on = !s.split_on;
                                cx.notify();
                            }
                        });
                    }
                    "h" => {
                        this.store.update(cx, |s, cx| {
                            if s.frame_full.is_hdr {
                                s.params.show_hdr_heatmap = !s.params.show_hdr_heatmap;
                                s.recompute_preview();
                                cx.notify();
                            }
                        });
                    }
                    "c" if ev.keystroke.modifiers.control => {
                        this.store.update(cx, |s, cx| {
                            s.copy_to_clipboard();
                            cx.notify();
                        });
                    }
                    "s" if ev.keystroke.modifiers.control => {
                        this.store.update(cx, |s, cx| {
                            s.save_png();
                            cx.notify();
                        });
                    }
                    _ => {}
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.clear_hover_tip(cx);
                    close_popover(&this.store, cx);
                    begin_native_window_drag(this.hwnd);
                }),
            )
            .child(zone_start)
            .child(zone_center)
            .child(zone_end)
    }
}

/// Independent click-through floating tooltip PopUp window (non-focus-stealing, 0 idle resources).
pub struct TooltipView {
    store: Entity<AppStore>,
    _sub: Subscription,
}

impl TooltipView {
    pub fn new(store: Entity<AppStore>, cx: &mut Context<Self>) -> Self {
        let sub = cx.observe(&store, |_, _, cx| cx.notify());
        Self { store, _sub: sub }
    }
}

impl Render for TooltipView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let text = self.store.read(cx).tooltip_text.unwrap_or("");
        div()
            .size_full()
            .rounded(px(8.0))
            .bg(rgb(0x181a1f))
            .border_1()
            .border_color(bar_border())
            .flex()
            .items_center()
            .justify_center()
            .px(px(10.0))
            .overflow_hidden()
            .whitespace_nowrap()
            .text_size(px(11.0))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(TEXT))
            .child(text)
    }
}

/// Independent floating popover window anchored beneath the Game Bar.
pub struct PopoverView {
    store: Entity<AppStore>,
    kind: PopoverKind,
    _sub: Subscription,
}

impl PopoverView {
    pub fn new(store: Entity<AppStore>, kind: PopoverKind, cx: &mut Context<Self>) -> Self {
        let sub = cx.observe(&store, |_, _, cx| cx.notify());
        Self {
            store,
            kind,
            _sub: sub,
        }
    }
}

impl Render for PopoverView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.kind {
            PopoverKind::Display => self.render_display_popover(cx),
            PopoverKind::HomeMenu => self.render_home_popover(cx),
        }
    }
}

impl PopoverView {
    fn render_display_popover(&self, cx: &mut Context<Self>) -> Div {
        let s = self.store.read(cx);
        let mut list = div()
            .size_full()
            .rounded(px(14.0))
            .bg(rgb(WIDGET_BG))
            .border_1()
            .border_color(bar_border())
            .p(px(8.0))
            .flex()
            .flex_col()
            .gap(px(3.0));

        for (idx, d) in s.displays.iter().enumerate() {
            let active = idx == s.selected_display;
            let name = d.friendly_name.clone();
            let orient = if d.is_portrait() { "V" } else { "H" };
            let badge = if d.is_hdr {
                format!(
                    "{}×{} {} · HDR {:.0}n",
                    d.width, d.height, orient, d.sdr_white_nits
                )
            } else {
                format!("{}×{} {} · SDR", d.width, d.height, orient)
            };
            let is_hdr = d.is_hdr;
            list = list.child(
                div()
                    .id(SharedString::from(format!("disp-opt-{}", idx)))
                    .h(px(34.0))
                    .px(px(10.0))
                    .rounded(px(9.0))
                    .bg(if active {
                        rgb(SURFACE_3)
                    } else {
                        rgba(0x00000000)
                    })
                    .flex()
                    .items_center()
                    .justify_between()
                    .cursor_pointer()
                    .hover(|s| s.bg(hover_overlay()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let store = this.store.clone();
                        cx.defer(move |cx| {
                            close_popover(&store, cx);
                            move_gamebar_to_display(&store, idx, cx);
                        });
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .w(px(7.0))
                                    .h(px(7.0))
                                    .rounded_full()
                                    .bg(rgb(if is_hdr { GREEN_BRIGHT } else { DIM })),
                            )
                            .child(div().text_xs().text_color(rgb(TEXT)).child(name)),
                    )
                    .child(div().text_size(px(11.0)).text_color(rgb(MUTED)).child(badge)),
            );
        }

        list.child(div().h(px(1.0)).my(px(3.0)).bg(subtle_border()))
            .child(
                div()
                    .id("disp-refresh")
                    .h(px(32.0))
                    .px(px(10.0))
                    .rounded(px(9.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(hover_overlay()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.store.update(cx, |s, cx| {
                            s.refresh_displays();
                            cx.notify();
                        });
                        let store = this.store.clone();
                        cx.defer(move |cx| close_popover(&store, cx));
                    }))
                    .child(svg_icon("icons/reset.svg", 13.0, MUTED))
                    .child(div().text_xs().text_color(rgb(MUTED)).child("Refresh displays")),
            )
    }

    fn render_home_popover(&self, cx: &mut Context<Self>) -> Div {
        let sc = self.store.read(cx).config.shortcuts.clone();

        let item = |id: &'static str, icon: &'static str, label: &'static str, kbd: String| {
            div()
                .id(id)
                .h(px(34.0))
                .px(px(10.0))
                .rounded(px(9.0))
                .flex()
                .items_center()
                .justify_between()
                .cursor_pointer()
                .hover(|s| s.bg(hover_overlay()))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(9.0))
                        .child(svg_icon(icon, 14.0, ICON_LIGHT))
                        .child(div().text_xs().text_color(rgb(TEXT)).child(label)),
                )
                .child(div().text_size(px(10.5)).text_color(rgb(DIM)).child(kbd))
        };

        div()
            .size_full()
            .rounded(px(14.0))
            .bg(rgb(WIDGET_BG))
            .border_1()
            .border_color(bar_border())
            .p(px(8.0))
            .flex()
            .flex_col()
            .gap(px(3.0))
            .child(
                item(
                    "menu-region",
                    "icons/region.svg",
                    "Snip region",
                    sc.region_snip.display_label(),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    let store = this.store.clone();
                    let delay = store.read(cx).config.default_delay_secs;
                    cx.defer(move |cx| {
                        close_popover(&store, cx);
                        trigger_capture(&store, delay, true, cx);
                    });
                })),
            )
            .child(
                item(
                    "menu-full",
                    "icons/camera.svg",
                    "Print full screen",
                    sc.full_screen.display_label(),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    let store = this.store.clone();
                    let delay = store.read(cx).config.default_delay_secs;
                    cx.defer(move |cx| {
                        close_popover(&store, cx);
                        trigger_capture(&store, delay, false, cx);
                    });
                })),
            )
            .child(div().h(px(1.0)).my(px(3.0)).bg(subtle_border()))
            .child(
                item("menu-copy", "icons/copy.svg", "Copy image", "Ctrl+C".into()).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.store.update(cx, |s, cx| {
                            s.copy_to_clipboard();
                            cx.notify();
                        });
                        let store = this.store.clone();
                        cx.defer(move |cx| close_popover(&store, cx));
                    }),
                ),
            )
            .child(
                item("menu-png", "icons/save.svg", "Save PNG", "Ctrl+S".into()).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.store.update(cx, |s, cx| {
                            s.save_png();
                            cx.notify();
                        });
                        let store = this.store.clone();
                        cx.defer(move |cx| close_popover(&store, cx));
                    }),
                ),
            )
            .child(
                item("menu-ab", "icons/split.svg", "Toggle A/B split", "B".into()).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.store.update(cx, |s, cx| {
                            if s.frame_full.is_hdr {
                                s.split_on = !s.split_on;
                                cx.notify();
                            }
                        });
                        let store = this.store.clone();
                        cx.defer(move |cx| close_popover(&store, cx));
                    }),
                ),
            )
            .child(
                item("menu-heat", "icons/heat.svg", "HDR heatmap", "H".into()).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.store.update(cx, |s, cx| {
                            if s.frame_full.is_hdr {
                                s.params.show_hdr_heatmap = !s.params.show_hdr_heatmap;
                                s.recompute_preview();
                                cx.notify();
                            }
                        });
                        let store = this.store.clone();
                        cx.defer(move |cx| close_popover(&store, cx));
                    }),
                ),
            )
            .child(div().h(px(1.0)).my(px(3.0)).bg(subtle_border()))
            .child(
                item(
                    "menu-shortcuts",
                    "icons/keyboard.svg",
                    "Shortcuts & settings",
                    sc.toggle_hud.display_label(),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    let store = this.store.clone();
                    cx.defer(move |cx| {
                        close_popover(&store, cx);
                        open_widget(&store, WidgetId::Settings, cx);
                    });
                })),
            )
    }
}
