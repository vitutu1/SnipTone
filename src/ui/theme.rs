use gpui::{Hsla, Pixels, Rgba, rgb, rgba};

#[inline]
pub fn px_f32(p: Pixels) -> f32 {
    f32::from(p)
}

// Windows 11 Game Bar Compact — Monochrome Graphite Palette

pub const BG_STAGE: u32 = 0x0d0e11;
pub const BAR_OUTER: u32 = 0x202226;
pub const BAR_CENTER: u32 = 0x2b2e34;
pub const HOME_BG: u32 = 0x3b3f47;
pub const HOME_HOVER: u32 = 0x464b54;

pub const WIDGET_BG: u32 = 0x1e2025;
pub const WIDGET_HEAD: u32 = 0x25282e;
pub const SURFACE_2: u32 = 0x2b2e35;
pub const SURFACE_3: u32 = 0x363a43;
pub const RECESS: u32 = 0x15171b;

pub const ACTIVE_BG: u32 = 0xd7dbe2;
pub const ACTIVE_BG_HOVER: u32 = 0xeaedf2;
pub const ACTIVE_FG: u32 = 0x15171b;

pub const TEXT: u32 = 0xeceff4;
pub const ICON_LIGHT: u32 = 0xe2e5ec;
pub const MUTED: u32 = 0x9aa1ae;
pub const DIM: u32 = 0x686f7d;

pub const GREEN: u32 = 0x16a34a;
pub const GREEN_BRIGHT: u32 = 0x22c55e;
pub const AMBER: u32 = 0xf59e0b;
pub const AMBER_BRIGHT: u32 = 0xfbbf24;

pub fn bar_border() -> Rgba {
    rgba(0xffffff16)
}

pub fn subtle_border() -> Rgba {
    rgba(0xffffff0e)
}

pub fn hover_overlay() -> Rgba {
    rgba(0xffffff17)
}

pub fn detent_color() -> Rgba {
    rgba(0xffffff40)
}

pub fn hist_sdr_bar() -> Rgba {
    rgba(0xd7dbe26b)
}

pub fn hist_hdr_bar() -> Rgba {
    rgba(0xf59e0bd9)
}

pub fn color_hex(hex: u32) -> Hsla {
    rgb(hex).into()
}
