use crate::display::DisplayRotation;
use rayon::prelude::*;

pub const HISTOGRAM_BINS: usize = 64;
pub const HIST_LOG_MIN: f32 = -1.0; // 0.1 nits
pub const HIST_LOG_MAX: f32 = 3.6; // ~4000 nits

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FrameStats {
    pub min_nits: f32,
    pub p50_nits: f32,
    pub p95_nits: f32,
    pub p99_nits: f32,
    pub p99_9_nits: f32,
    pub max_nits: f32,
    pub pct_above_sdr_white: f32,
    pub pct_wide_gamut_negative: f32,
    /// Normalized [0..1] display histogram across `HIST_LOG_MIN..HIST_LOG_MAX` log10(nits).
    pub histogram: [f32; HISTOGRAM_BINS],
}

impl Default for FrameStats {
    fn default() -> Self {
        Self {
            min_nits: 0.0,
            p50_nits: 0.0,
            p95_nits: 0.0,
            p99_nits: 0.0,
            p99_9_nits: 0.0,
            max_nits: 80.0,
            pct_above_sdr_white: 0.0,
            pct_wide_gamut_negative: 0.0,
            histogram: [0.0; HISTOGRAM_BINS],
        }
    }
}

/// Platform-agnostic captured image buffer (linear scRGB for HDR, sRGB [0..1] for SDR).
#[derive(Debug, Clone)]
pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    /// True if `rgb` contains linear scRGB float values (1.0 = 80 nits, may exceed 1.0 or be < 0.0).
    /// False if `rgb` contains sRGB gamma-encoded [0.0, 1.0] values from SDR capture.
    pub is_hdr: bool,
    pub sdr_white_nits: f32,
    pub monitor_max_nits: f32,
    pub source_label: String,
    /// Interleaved linear scRGB (HDR) or sRGB (SDR) pixels: `[R, G, B]`.
    pub rgb: Vec<[f32; 3]>,
    pub stats: FrameStats,
}

impl CapturedFrame {
    /// Rotates the captured buffer clockwise by 90°, 180°, or 270° in parallel.
    /// Used when DXGI Desktop Duplication returns an unrotated landscape surface for a portrait display.
    pub fn rotate(&self, rotation: DisplayRotation) -> Self {
        if rotation == DisplayRotation::Identity || self.width == 0 || self.height == 0 {
            return self.clone();
        }

        let sw = self.width as usize;
        let sh = self.height as usize;

        let (dw, dh) = match rotation {
            DisplayRotation::Identity | DisplayRotation::Rotate180 => (sw, sh),
            DisplayRotation::Rotate90 | DisplayRotation::Rotate270 => (sh, sw),
        };

        let mut dst = vec![[0.0f32; 3]; dw * dh];
        dst.par_chunks_mut(dw).enumerate().for_each(|(dy, row)| {
            for (dx, out_px) in row.iter_mut().enumerate() {
                let (sx, sy) = match rotation {
                    DisplayRotation::Identity => (dx, dy),
                    DisplayRotation::Rotate90 => (dy, sh - 1 - dx),
                    DisplayRotation::Rotate180 => (sw - 1 - dx, sh - 1 - dy),
                    DisplayRotation::Rotate270 => (sw - 1 - dy, dx),
                };
                *out_px = self.rgb[sy * sw + sx];
            }
        });

        Self {
            width: dw as u32,
            height: dh as u32,
            is_hdr: self.is_hdr,
            sdr_white_nits: self.sdr_white_nits,
            monitor_max_nits: self.monitor_max_nits,
            source_label: self.source_label.clone(),
            rgb: dst,
            stats: self.stats.clone(),
        }
    }

    /// Generates a fast downscaled copy capped on its longest edge (`max_dim`) so both landscape
    /// (e.g. 2560x1440 -> 1280x720) and portrait (e.g. 1080x1920 -> 720x1280) run at 120 FPS.
    pub fn downscale_for_preview(&self, max_dim: u32) -> Self {
        let longest = self.width.max(self.height);
        if longest <= max_dim || self.width == 0 || self.height == 0 {
            return self.clone();
        }

        let scale = max_dim as f32 / longest as f32;
        let dst_w = ((self.width as f32 * scale).round() as usize).max(1);
        let dst_h = ((self.height as f32 * scale).round() as usize).max(1);
        let src_w = self.width as usize;
        let src_h = self.height as usize;

        let mut dst = vec![[0.0f32; 3]; dst_w * dst_h];
        let x_scale = src_w as f32 / dst_w as f32;
        let y_scale = src_h as f32 / dst_h as f32;

        dst.par_chunks_mut(dst_w).enumerate().for_each(|(dy, row)| {
            let sy0 = ((dy as f32) * y_scale) as usize;
            let sy1 = (((dy + 1) as f32) * y_scale).ceil().min(src_h as f32) as usize;
            let sy1 = sy1.max(sy0 + 1).min(src_h);

            for (dx, out_px) in row.iter_mut().enumerate() {
                let sx0 = ((dx as f32) * x_scale) as usize;
                let sx1 = (((dx + 1) as f32) * x_scale).ceil().min(src_w as f32) as usize;
                let sx1 = sx1.max(sx0 + 1).min(src_w);

                let mut sum = [0.0f32; 3];
                let mut count = 0u32;
                for sy in sy0..sy1 {
                    let row_off = sy * src_w;
                    for sx in sx0..sx1 {
                        let p = self.rgb[row_off + sx];
                        sum[0] += p[0];
                        sum[1] += p[1];
                        sum[2] += p[2];
                        count += 1;
                    }
                }
                let inv = 1.0 / (count.max(1) as f32);
                *out_px = [sum[0] * inv, sum[1] * inv, sum[2] * inv];
            }
        });

        Self {
            width: dst_w as u32,
            height: dst_h as u32,
            is_hdr: self.is_hdr,
            sdr_white_nits: self.sdr_white_nits,
            monitor_max_nits: self.monitor_max_nits,
            source_label: self.source_label.clone(),
            rgb: dst,
            stats: self.stats.clone(),
        }
    }

    /// Crops a normalized rectangle `(u0, v0, u1, v1)` in `[0.0, 1.0]` directly from the linear HDR
    /// buffer with zero loss and recomputes luminance telemetry.
    pub fn crop_normalized(&self, u0: f32, v0: f32, u1: f32, v1: f32) -> Option<Self> {
        let x0 = ((u0.clamp(0.0, 1.0) * self.width as f32).floor() as u32).min(self.width);
        let y0 = ((v0.clamp(0.0, 1.0) * self.height as f32).floor() as u32).min(self.height);
        let x1 = ((u1.clamp(0.0, 1.0) * self.width as f32).ceil() as u32).min(self.width);
        let y1 = ((v1.clamp(0.0, 1.0) * self.height as f32).ceil() as u32).min(self.height);

        let cw = x1.saturating_sub(x0);
        let ch = y1.saturating_sub(y0);
        if cw < 8 || ch < 8 {
            return None;
        }

        let src_w = self.width as usize;
        let dst_w = cw as usize;
        let mut rgb = vec![[0.0f32; 3]; dst_w * (ch as usize)];

        rgb.par_chunks_mut(dst_w).enumerate().for_each(|(dy, row)| {
            let src_start = ((y0 as usize) + dy) * src_w + (x0 as usize);
            row.copy_from_slice(&self.rgb[src_start..src_start + dst_w]);
        });

        let stats = compute_frame_stats(&rgb, self.is_hdr, self.sdr_white_nits);
        Some(Self {
            width: cw,
            height: ch,
            is_hdr: self.is_hdr,
            sdr_white_nits: self.sdr_white_nits,
            monitor_max_nits: self.monitor_max_nits,
            source_label: format!("{} (Region)", self.source_label),
            rgb,
            stats,
        })
    }

    /// Generates a rich synthetic HDR scene (twilight skyline + ~1800-nit solar flare +
    /// wide-gamut neons + 100% SDR white HUD card) for immediate evaluation.
    pub fn synthetic_hdr_test_pattern(width: u32, height: u32, sdr_white_nits: f32) -> Self {
        let sdr_ref = sdr_white_nits / 80.0;
        let w = width as usize;
        let h = height as usize;
        let mut rgb = vec![[0.0f32; 3]; w * h];

        let b_count = 26usize;
        let b_heights: Vec<f32> = (0..b_count)
            .map(|i| {
                let s = ((i as f32) * 12.9898).sin() * 43758.5453;
                0.08 + (s - s.floor()) * 0.24
            })
            .collect();

        rgb.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            let v = y as f32 / (h.max(1) as f32);
            let aspect = w as f32 / (h.max(1) as f32);
            let horizon = 0.56f32;

            for (x, px) in row.iter_mut().enumerate() {
                let u = x as f32 / (w.max(1) as f32);
                let (mut r, mut g, mut b);

                if v < horizon {
                    let sky_t = v / horizon;
                    r = (0.015 + 0.16 * sky_t.powf(2.4)) * sdr_ref;
                    g = (0.025 + 0.09 * sky_t.powf(2.0)) * sdr_ref;
                    b = (0.085 + 0.14 * sky_t.powf(1.4)) * sdr_ref;

                    let dx = (u - 0.62) * aspect;
                    let dy = v - 0.34;
                    let dist = (dx * dx + dy * dy).sqrt();
                    let sun_core = (-dist * dist * 260.0).exp() * 6.5 * sdr_ref;
                    let sun_halo = (-dist * dist * 22.0).exp() * 1.45 * sdr_ref;
                    let flare = (-dy.abs() * 45.0).exp() * (-dx.abs() * 4.5).exp() * 1.15 * sdr_ref;

                    r += sun_core * 1.08 + sun_halo * 1.12 + flare * 1.05;
                    g += sun_core * 0.90 + sun_halo * 0.55 + flare * 0.65;
                    b += sun_core * 0.62 + sun_halo * 0.22 + flare * 0.30;

                    let bi = ((u * b_count as f32) as usize).min(b_count - 1);
                    let b_top = horizon - b_heights[bi];
                    if v > b_top {
                        r = 0.014 * sdr_ref;
                        g = 0.018 * sdr_ref;
                        b = 0.030 * sdr_ref;

                        let cell_x = u * (b_count as f32) - (bi as f32);
                        let wx = (cell_x * 5.0).floor();
                        let wy = ((v - b_top) * 58.0).floor();
                        let fx = cell_x * 5.0 - wx;
                        let fy = (v - b_top) * 58.0 - wy;
                        let hv = ((bi as f32) * 78.23 + wx * 19.1 + wy * 37.7).sin() * 43758.545;
                        let frac = hv - hv.floor();
                        if fx > 0.24 && fx < 0.76 && fy > 0.25 && fy < 0.72 && frac > 0.62 {
                            let hot = if frac > 0.91 {
                                3.8 * sdr_ref
                            } else {
                                0.85 * sdr_ref
                            };
                            if bi % 3 == 0 {
                                r += hot * 0.08;
                                g += hot * 0.92;
                                b += hot * 1.18;
                            } else if bi % 4 == 0 {
                                r += hot * 1.25;
                                g += hot * -0.08;
                                b += hot * 0.55;
                            } else {
                                r += hot * 1.05;
                                g += hot * 0.72;
                                b += hot * 0.28;
                            }
                        }
                    }
                } else {
                    let wt = (v - horizon) / (1.0 - horizon);
                    let wave = (v * 140.0 + (u * 35.0).sin() * 2.5).sin() * 0.012 * (0.3 + wt);
                    let ru = u + wave;

                    r = (0.018 + 0.04 * (1.0 - wt)) * sdr_ref;
                    g = (0.024 + 0.05 * (1.0 - wt)) * sdr_ref;
                    b = (0.052 + 0.08 * (1.0 - wt)) * sdr_ref;

                    let rx = (ru - 0.62).abs();
                    let spread = 0.028 + wt * 0.16;
                    let shimmer = 0.55 + 0.45 * (v * 230.0 + u * 60.0).sin();
                    let sun_refl = (-(rx * rx) / (spread * spread)).exp()
                        * (1.0 - wt * 0.55)
                        * shimmer
                        * 4.4
                        * sdr_ref;
                    r += sun_refl * 1.08;
                    g += sun_refl * 0.74;
                    b += sun_refl * 0.34;

                    let cx = (ru - 0.24).abs();
                    let cyan_refl = (-(cx * cx) / (0.07 * 0.07)).exp()
                        * (1.0 - wt * 0.6)
                        * shimmer
                        * 2.6
                        * sdr_ref;
                    r += cyan_refl * 0.05;
                    g += cyan_refl * 0.88;
                    b += cyan_refl * 1.2;

                    let mx = (ru - 0.41).abs();
                    let mag_refl = (-(mx * mx) / (0.055 * 0.055)).exp()
                        * (1.0 - wt * 0.65)
                        * shimmer
                        * 2.4
                        * sdr_ref;
                    r += mag_refl * 1.15;
                    g += mag_refl * -0.05;
                    b += mag_refl * 0.52;
                }

                // Crisp SDR 100% paper-white HUD block in bottom-left
                if u > 0.045 && u < 0.255 && v > 0.77 && v < 0.93 {
                    let lx = (u - 0.045) / 0.21;
                    let ly = (v - 0.77) / 0.16;
                    r = r * 0.2 + 0.035 * sdr_ref;
                    g = g * 0.2 + 0.042 * sdr_ref;
                    b = b * 0.2 + 0.055 * sdr_ref;
                    if ly < 0.04 {
                        r = 0.85 * sdr_ref;
                        g = 0.88 * sdr_ref;
                        b = 0.95 * sdr_ref;
                    }
                    if ly > 0.22 && ly < 0.36 && lx > 0.08 && lx < 0.88 {
                        r = sdr_ref;
                        g = sdr_ref;
                        b = sdr_ref;
                    }
                    if ly > 0.50 && ly < 0.64 && lx > 0.08 && lx < 0.68 {
                        r = 0.08 * sdr_ref;
                        g = 0.72 * sdr_ref;
                        b = 0.92 * sdr_ref;
                    }
                    if ly > 0.74 && ly < 0.84 && lx > 0.08 && lx < 0.52 {
                        r = 0.55 * sdr_ref;
                        g = 0.58 * sdr_ref;
                        b = 0.64 * sdr_ref;
                    }
                }

                *px = [r, g, b];
            }
        });

        let stats = compute_frame_stats(&rgb, true, sdr_white_nits);
        Self {
            width,
            height,
            is_hdr: true,
            sdr_white_nits,
            monitor_max_nits: 1200.0,
            source_label: "Synthetic HDR Scene".to_string(),
            rgb,
            stats,
        }
    }
}

#[inline]
fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// Computes luminance percentiles and a 64-bin display histogram in a single parallel pass.
pub fn compute_frame_stats(rgb: &[[f32; 3]], is_hdr: bool, sdr_white_nits: f32) -> FrameStats {
    if rgb.is_empty() {
        return FrameStats::default();
    }

    const BINS: usize = 2048;
    const LOG_MIN: f32 = -4.0;
    const LOG_MAX: f32 = 4.5;
    const LOG_SPAN: f32 = LOG_MAX - LOG_MIN;
    const DISPLAY_SPAN: f32 = HIST_LOG_MAX - HIST_LOG_MIN;

    let sdr_ref_scrgb = (sdr_white_nits / 80.0).max(0.1);

    struct Acc {
        hist: Vec<u64>,
        disp_hist: [u64; HISTOGRAM_BINS],
        min_nits: f32,
        max_nits: f32,
        above_sdr: u64,
        wide_gamut: u64,
    }

    let reduced = rgb
        .par_chunks(16384)
        .fold(
            || Acc {
                hist: vec![0u64; BINS],
                disp_hist: [0u64; HISTOGRAM_BINS],
                min_nits: f32::MAX,
                max_nits: 0.0,
                above_sdr: 0,
                wide_gamut: 0,
            },
            |mut acc, chunk| {
                for &[r, g, b] in chunk {
                    if is_hdr && (r < -0.001 || g < -0.001 || b < -0.001) {
                        acc.wide_gamut += 1;
                    }

                    let nits = if is_hdr {
                        let y = (0.2126 * r + 0.7152 * g + 0.0722 * b).max(0.0);
                        if r > sdr_ref_scrgb * 1.01
                            || g > sdr_ref_scrgb * 1.01
                            || b > sdr_ref_scrgb * 1.01
                        {
                            acc.above_sdr += 1;
                        }
                        y * 80.0
                    } else {
                        let lr = srgb_to_linear(r.clamp(0.0, 1.0));
                        let lg = srgb_to_linear(g.clamp(0.0, 1.0));
                        let lb = srgb_to_linear(b.clamp(0.0, 1.0));
                        (0.2126 * lr + 0.7152 * lg + 0.0722 * lb) * sdr_white_nits
                    };

                    if nits < acc.min_nits {
                        acc.min_nits = nits;
                    }
                    if nits > acc.max_nits {
                        acc.max_nits = nits;
                    }

                    let log_v = nits.max(1e-4).log10();
                    let norm = ((log_v - LOG_MIN) / LOG_SPAN).clamp(0.0, 1.0);
                    let bin = ((norm * (BINS - 1) as f32) as usize).min(BINS - 1);
                    acc.hist[bin] += 1;

                    let disp_norm = ((nits.max(0.1).log10() - HIST_LOG_MIN) / DISPLAY_SPAN)
                        .clamp(0.0, 1.0);
                    let dbin = ((disp_norm * (HISTOGRAM_BINS - 1) as f32) as usize)
                        .min(HISTOGRAM_BINS - 1);
                    acc.disp_hist[dbin] += 1;
                }
                acc
            },
        )
        .reduce(
            || Acc {
                hist: vec![0u64; BINS],
                disp_hist: [0u64; HISTOGRAM_BINS],
                min_nits: f32::MAX,
                max_nits: 0.0,
                above_sdr: 0,
                wide_gamut: 0,
            },
            |mut a, b| {
                for i in 0..BINS {
                    a.hist[i] += b.hist[i];
                }
                for i in 0..HISTOGRAM_BINS {
                    a.disp_hist[i] += b.disp_hist[i];
                }
                a.min_nits = a.min_nits.min(b.min_nits);
                a.max_nits = a.max_nits.max(b.max_nits);
                a.above_sdr += b.above_sdr;
                a.wide_gamut += b.wide_gamut;
                a
            },
        );

    let total = rgb.len() as u64;
    let percentile = |p: f64| -> f32 {
        let target = ((total as f64) * p).ceil() as u64;
        let mut cum = 0u64;
        for (i, &count) in reduced.hist.iter().enumerate() {
            cum += count;
            if cum >= target {
                let frac = (i as f32 + 0.5) / (BINS as f32);
                return 10.0_f32.powf(LOG_MIN + frac * LOG_SPAN);
            }
        }
        reduced.max_nits
    };

    let hist_max = reduced.disp_hist.iter().copied().max().unwrap_or(1).max(1) as f32;
    let mut histogram = [0.0f32; HISTOGRAM_BINS];
    for i in 0..HISTOGRAM_BINS {
        histogram[i] = (reduced.disp_hist[i] as f32 / hist_max).powf(0.45);
    }

    FrameStats {
        min_nits: if reduced.min_nits.is_finite() {
            reduced.min_nits
        } else {
            0.0
        },
        p50_nits: percentile(0.50),
        p95_nits: percentile(0.95),
        p99_nits: percentile(0.99),
        p99_9_nits: percentile(0.999),
        max_nits: reduced.max_nits,
        pct_above_sdr_white: (reduced.above_sdr as f32 / total as f32) * 100.0,
        pct_wide_gamut_negative: (reduced.wide_gamut as f32 / total as f32) * 100.0,
        histogram,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotate_270_swaps_dimensions_and_preserves_corners() {
        // 3x2 landscape frame with unique pixels
        let frame = CapturedFrame {
            width: 3,
            height: 2,
            is_hdr: false,
            sdr_white_nits: 80.0,
            monitor_max_nits: 80.0,
            source_label: "test".into(),
            rgb: vec![
                [1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [3.0, 0.0, 0.0],
                [4.0, 0.0, 0.0], [5.0, 0.0, 0.0], [6.0, 0.0, 0.0],
            ],
            stats: FrameStats::default(),
        };

        let rot = frame.rotate(DisplayRotation::Rotate270);
        assert_eq!((rot.width, rot.height), (2, 3));
        // Top-left of 270° rotated (CCW 90°) comes from top-right (pixel 3.0)
        assert_eq!(rot.rgb[0][0], 3.0);
        // Bottom-left comes from top-left (pixel 1.0)
        assert_eq!(rot.rgb[2 * 2][0], 1.0);
    }

    #[test]
    fn downscale_for_preview_caps_longest_edge_on_portrait() {
        let frame = CapturedFrame {
            width: 1080,
            height: 1920,
            is_hdr: false,
            sdr_white_nits: 80.0,
            monitor_max_nits: 80.0,
            source_label: "portrait".into(),
            rgb: vec![[0.5, 0.5, 0.5]; 1080 * 1920],
            stats: FrameStats::default(),
        };
        let preview = frame.downscale_for_preview(1280);
        assert_eq!(preview.width, 720);
        assert_eq!(preview.height, 1280);
    }
}
