use crate::frame::CapturedFrame;
use rayon::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TonemapOperator {
    /// Preserves SDR content [0, shoulder] 1:1 and smoothly rolls off HDR highlights above it.
    /// Solves both Snipping Tool's washed-out SDR look and OBS's hard highlight clipping.
    HybridSdrSafe,
    /// ITU-R BT.2390 EETF in perceptual PQ (ST 2084) space with Hermite shoulder spline.
    Bt2390,
    /// Khronos PBR Neutral: preserves base colors faithfully with neutral highlight desaturation.
    PbrNeutral,
    /// AgX filmic (Blender 4 / Troy Sobotka): eliminates hue skew on intense HDR lights/neons.
    AgX,
    /// ACES RRT+ODT fitted curve (Stephen Hill) anchored to SDR paper white.
    AcesFitted,
    /// Extended Reinhard on luminance (preserves chromaticity better than per-channel Reinhard).
    ReinhardLuminance,
    /// Hard-clip to SDR paper white + sRGB OETF (same baseline method as OBS / MagestiUA).
    WindowsClip,
}

impl TonemapOperator {
    pub const ALL: &'static [TonemapOperator] = &[
        TonemapOperator::HybridSdrSafe,
        TonemapOperator::Bt2390,
        TonemapOperator::PbrNeutral,
        TonemapOperator::AgX,
        TonemapOperator::AcesFitted,
        TonemapOperator::ReinhardLuminance,
        TonemapOperator::WindowsClip,
    ];

    #[allow(dead_code)]
    pub fn label(self) -> &'static str {
        match self {
            Self::HybridSdrSafe => "Hybrid SDR-Safe",
            Self::Bt2390 => "ITU-R BT.2390 (PQ)",
            Self::PbrNeutral => "Khronos PBR Neutral",
            Self::AgX => "AgX Filmic",
            Self::AcesFitted => "ACES Fitted",
            Self::ReinhardLuminance => "Reinhard Extended",
            Self::WindowsClip => "Windows / OBS Clip",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::HybridSdrSafe => "Hybrid",
            Self::Bt2390 => "BT.2390",
            Self::PbrNeutral => "PBR Neutral",
            Self::AgX => "AgX",
            Self::AcesFitted => "ACES",
            Self::ReinhardLuminance => "Reinhard",
            Self::WindowsClip => "Win Clip",
        }
    }

    #[allow(dead_code)]
    pub fn description(self) -> &'static str {
        match self {
            Self::HybridSdrSafe => {
                "Keeps SDR UI 1:1 up to the shoulder and smoothly rolls off HDR highlights while preserving hue."
            }
            Self::Bt2390 => {
                "Broadcast-grade ITU-R BT.2390 EETF in perceptual PQ (ST 2084) space."
            }
            Self::PbrNeutral => {
                "Faithful to base SDR colors up to ~0.8 with neutral highlight desaturation."
            }
            Self::AgX => {
                "Modern filmic curve with no hue skew on bright HDR neons, fire, and explosions."
            }
            Self::AcesFitted => {
                "ACES RRT+ODT fitted curve (Stephen Hill) anchored to SDR paper white."
            }
            Self::ReinhardLuminance => {
                "Luminance-based extended Reinhard using Peak Lum as the white point."
            }
            Self::WindowsClip => {
                "OBS / MagestiUA baseline: divides by SDR white and hard-clips above 1.0."
            }
        }
    }

    pub fn uses_shoulder(self) -> bool {
        matches!(self, Self::HybridSdrSafe | Self::PbrNeutral)
    }

    pub fn uses_max_luminance(self) -> bool {
        matches!(
            self,
            Self::HybridSdrSafe | Self::Bt2390 | Self::ReinhardLuminance
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GamutMapping {
    /// Desaturates out-of-gamut scRGB negative channels toward achromatic luminance (preserves hue).
    SoftLuminance,
    /// Clamps negative channels directly to 0.0.
    HardClamp,
}

impl GamutMapping {
    #[allow(dead_code)]
    pub fn label(self) -> &'static str {
        match self {
            Self::SoftLuminance => "Soft (Hue-Safe)",
            Self::HardClamp => "Hard Clamp",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TonemapParams {
    pub operator: TonemapOperator,
    /// Windows SDR white reference in nits (e.g. 203..250 nits). `1.0` in scRGB = 80 nits.
    pub sdr_white_nits: f32,
    /// Fine exposure offset in stops (-2.0 .. +2.0 EV).
    pub exposure_ev: f32,
    /// Source peak luminance reference in nits for curves with shoulder/white point (200..4000 nits).
    pub max_luminance_nits: f32,
    /// Where SDR-preserving compression begins in linear SDR space (0.50..0.95).
    pub shoulder_start: f32,
    /// Extra highlight desaturation toward white for extreme HDR values (0.0..1.0).
    pub highlight_desat: f32,
    /// How negative scRGB (wide color gamut P3/BT.2020) values are mapped into sRGB.
    pub gamut_mapping: GamutMapping,
    /// When enabled, overlays a false-color heatmap over pixels exceeding SDR white (> 1.0).
    pub show_hdr_heatmap: bool,
}

impl Default for TonemapParams {
    fn default() -> Self {
        Self {
            operator: TonemapOperator::HybridSdrSafe,
            sdr_white_nits: 240.0,
            exposure_ev: 0.0,
            max_luminance_nits: 1000.0,
            shoulder_start: 0.78,
            highlight_desat: 0.35,
            gamut_mapping: GamutMapping::SoftLuminance,
            show_hdr_heatmap: false,
        }
    }
}

#[inline]
fn luminance(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

#[inline]
pub fn linear_to_srgb(v: f32) -> f32 {
    let x = v.clamp(0.0, 1.0);
    if x <= 0.0031308 {
        12.92 * x
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    }
}

#[inline]
fn float_to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// Maps negative scRGB (wide gamut P3/BT.2020) values into the sRGB gamut.
#[inline]
fn map_negative_gamut(c: [f32; 3], mode: GamutMapping) -> [f32; 3] {
    let min_c = c[0].min(c[1]).min(c[2]);
    if min_c >= 0.0 {
        return c;
    }

    match mode {
        GamutMapping::HardClamp => [c[0].max(0.0), c[1].max(0.0), c[2].max(0.0)],
        GamutMapping::SoftLuminance => {
            let y = luminance(c);
            if y <= 1e-6 {
                return [0.0, 0.0, 0.0];
            }
            // Pull minimally toward (y, y, y) so the minimum channel lands at 0.0 while keeping luminance == y.
            let t = (y / (y - min_c)).clamp(0.0, 1.0);
            [
                (y + t * (c[0] - y)).max(0.0),
                (y + t * (c[1] - y)).max(0.0),
                (y + t * (c[2] - y)).max(0.0),
            ]
        }
    }
}

/// Ensures individual channels <= 1.0 after luminance tone mapping by desaturating toward `y_out`
/// (preserving luminance and avoiding ugly single-channel clipping hue shifts), plus optional
/// natural highlight desaturation for extreme HDR intensities.
#[inline]
fn finish_luminance_mapped(
    c: [f32; 3],
    y_in: f32,
    y_out: f32,
    highlight_desat: f32,
) -> [f32; 3] {
    if y_in <= 1e-7 || y_out <= 0.0 {
        return [0.0, 0.0, 0.0];
    }

    let scale = y_out / y_in;
    let mut mapped = [c[0] * scale, c[1] * scale, c[2] * scale];

    // Optional gentle perceptual desaturation for highlights above SDR paper white (y_in > 1.0)
    if highlight_desat > 0.0 && y_in > 1.0 {
        let excess = (y_in - 1.0).max(0.0);
        let desat_t = (1.0 - 1.0 / (1.0 + excess * highlight_desat * 0.65)).clamp(0.0, 0.85);
        mapped = [
            mapped[0] + (y_out - mapped[0]) * desat_t,
            mapped[1] + (y_out - mapped[1]) * desat_t,
            mapped[2] + (y_out - mapped[2]) * desat_t,
        ];
    }

    // Softly pull any channel still > 1.0 toward achromatic y_out so no single channel hard-clips
    let max_c = mapped[0].max(mapped[1]).max(mapped[2]);
    if max_c > 1.0 {
        let target_y = y_out.min(1.0);
        let denom = (max_c - target_y).max(1e-6);
        let t = ((1.0 - target_y) / denom).clamp(0.0, 1.0);
        mapped = [
            target_y + t * (mapped[0] - target_y),
            target_y + t * (mapped[1] - target_y),
            target_y + t * (mapped[2] - target_y),
        ];
    }

    [
        mapped[0].clamp(0.0, 1.0),
        mapped[1].clamp(0.0, 1.0),
        mapped[2].clamp(0.0, 1.0),
    ]
}

// ── ST 2084 PQ OETF / EOTF (for ITU-R BT.2390) ───────────────────────────────

const PQ_M1: f32 = 2610.0 / 16384.0;
const PQ_M2: f32 = (2523.0 / 4096.0) * 128.0;
const PQ_C1: f32 = 3424.0 / 4096.0;
const PQ_C2: f32 = (2413.0 / 4096.0) * 32.0;
const PQ_C3: f32 = (2392.0 / 4096.0) * 32.0;

#[inline]
fn pq_oetf(linear_10k: f32) -> f32 {
    let y = linear_10k.clamp(0.0, 1.0);
    let ym1 = y.powf(PQ_M1);
    ((PQ_C1 + PQ_C2 * ym1) / (1.0 + PQ_C3 * ym1)).powf(PQ_M2)
}

#[inline]
fn pq_eotf(pq: f32) -> f32 {
    let e = pq.clamp(0.0, 1.0);
    let em2 = e.powf(1.0 / PQ_M2);
    let num = (em2 - PQ_C1).max(0.0);
    let den = (PQ_C2 - PQ_C3 * em2).max(1e-6);
    (num / den).powf(1.0 / PQ_M1)
}

/// Precomputed per-frame constants so inner pixel loops avoid redundant `powf` / divisions.
#[derive(Debug, Clone, Copy)]
struct PreparedParams {
    params: TonemapParams,
    factor: f32,
    hybrid_s: f32,
    hybrid_h: f32,
    hybrid_p: f32,
    hybrid_inv_hp: f32,
    bt_sdr_white: f32,
    bt_e_max: f32,
    bt_max_lum: f32,
    bt_ks: f32,
    bt_inv_one_minus_ks: f32,
    pbr_start: f32,
    pbr_desat: f32,
    reinhard_inv_lwhite_sq: f32,
}

impl PreparedParams {
    fn new(params: TonemapParams) -> Self {
        let sdr_white = params.sdr_white_nits.max(40.0);
        let sdr_scale = sdr_white / 80.0;
        let factor = 2.0_f32.powf(params.exposure_ev) / sdr_scale;

        let hybrid_s = params.shoulder_start.clamp(0.45, 0.95);
        let hybrid_h = 1.0 - hybrid_s;
        let max_ratio = (params.max_luminance_nits / sdr_white).max(1.25);
        let hybrid_p = (4.0 / max_ratio).clamp(0.35, 3.0);
        let hybrid_inv_hp = 1.0 / (hybrid_h * hybrid_p);

        let max_nits = params.max_luminance_nits.max(sdr_white * 1.1);
        let bt_e_max = pq_oetf(max_nits / 10000.0).max(1e-5);
        let target_max = pq_oetf(sdr_white / 10000.0);
        let bt_max_lum = (target_max / bt_e_max).clamp(0.05, 1.0);
        let bt_ks = (1.5 * bt_max_lum - 0.5).clamp(0.0, bt_max_lum);
        let bt_inv_one_minus_ks = if (1.0 - bt_ks) > 1e-5 {
            1.0 / (1.0 - bt_ks)
        } else {
            0.0
        };

        let pbr_start = (params.shoulder_start.clamp(0.45, 0.95) - 0.04).max(0.40);
        let pbr_desat = 0.05 + params.highlight_desat * 0.40;

        let l_white = (params.max_luminance_nits / sdr_white).max(1.1);
        let reinhard_inv_lwhite_sq = 1.0 / (l_white * l_white);

        Self {
            params,
            factor,
            hybrid_s,
            hybrid_h,
            hybrid_p,
            hybrid_inv_hp,
            bt_sdr_white: sdr_white,
            bt_e_max,
            bt_max_lum,
            bt_ks,
            bt_inv_one_minus_ks,
            pbr_start,
            pbr_desat,
            reinhard_inv_lwhite_sq,
        }
    }
}

// ── Individual Tonemapping Operators (Linear SDR-normalized -> Linear [0,1]) ─

#[inline]
fn op_hybrid_sdr_safe(c: [f32; 3], p: &PreparedParams) -> [f32; 3] {
    let y = luminance(c);
    let y_out = if y <= p.hybrid_s {
        y
    } else {
        let d = y - p.hybrid_s;
        // Power-rational C^∞ curve with exact derivative f'(s) == 1.0 for every p > 0:
        // f(d) = h * (1 - (1 + d / (h * p))^(-p))
        p.hybrid_s + p.hybrid_h * (1.0 - (1.0 + d * p.hybrid_inv_hp).powf(-p.hybrid_p))
    };

    finish_luminance_mapped(c, y, y_out, p.params.highlight_desat)
}

#[inline]
fn op_bt2390(c: [f32; 3], p: &PreparedParams) -> [f32; 3] {
    let y_norm = luminance(c);
    if y_norm <= 1e-7 {
        return [0.0, 0.0, 0.0];
    }

    let y_nits = y_norm * p.bt_sdr_white;
    let e1 = (pq_oetf(y_nits / 10000.0) / p.bt_e_max).clamp(0.0, 1.0);

    let e2 = if e1 < p.bt_ks || p.bt_inv_one_minus_ks == 0.0 {
        e1
    } else {
        let t = ((e1 - p.bt_ks) * p.bt_inv_one_minus_ks).clamp(0.0, 1.0);
        let t2 = t * t;
        let t3 = t2 * t;
        (2.0 * t3 - 3.0 * t2 + 1.0) * p.bt_ks
            + (t3 - 2.0 * t2 + t) * (1.0 - p.bt_ks)
            + (-2.0 * t3 + 3.0 * t2) * p.bt_max_lum
    };

    let y_out_nits = pq_eotf((e2 * p.bt_e_max).clamp(0.0, 1.0)) * 10000.0;
    let y_out = (y_out_nits / p.bt_sdr_white).clamp(0.0, 1.0);

    finish_luminance_mapped(c, y_norm, y_out, p.params.highlight_desat)
}

#[inline]
fn op_pbr_neutral(c: [f32; 3], p: &PreparedParams) -> [f32; 3] {
    let x = c[0].min(c[1]).min(c[2]);
    let offset = if x < 0.08 {
        x - 6.25 * x * x
    } else {
        0.04
    };
    let mut color = [
        (c[0] - offset).max(0.0),
        (c[1] - offset).max(0.0),
        (c[2] - offset).max(0.0),
    ];

    let peak = color[0].max(color[1]).max(color[2]);
    if peak < p.pbr_start {
        return [
            color[0].clamp(0.0, 1.0),
            color[1].clamp(0.0, 1.0),
            color[2].clamp(0.0, 1.0),
        ];
    }

    let d = 1.0 - p.pbr_start;
    let new_peak = 1.0 - (d * d) / (peak + d - p.pbr_start);
    let scale = new_peak / peak;
    color = [color[0] * scale, color[1] * scale, color[2] * scale];

    let g = 1.0 - 1.0 / (p.pbr_desat * (peak - new_peak) + 1.0);
    [
        (color[0] + (new_peak - color[0]) * g).clamp(0.0, 1.0),
        (color[1] + (new_peak - color[1]) * g).clamp(0.0, 1.0),
        (color[2] + (new_peak - color[2]) * g).clamp(0.0, 1.0),
    ]
}

#[inline]
fn agx_default_contrast_approx(x: f32) -> f32 {
    let x2 = x * x;
    let x4 = x2 * x2;
    15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x
        - 0.00232
}

#[inline]
fn op_agx(c: [f32; 3]) -> [f32; 3] {
    // AgX inset matrix (linear sRGB -> AgX log working space)
    let r = 0.842479062253094 * c[0] + 0.0784335999999992 * c[1] + 0.0792237451477643 * c[2];
    let g = 0.0423282422610123 * c[0] + 0.878468636469772 * c[1] + 0.0791661274605434 * c[2];
    let b = 0.0423756549057051 * c[0] + 0.0784336 * c[1] + 0.879142973793104 * c[2];

    const MIN_EV: f32 = -12.47393;
    const MAX_EV: f32 = 4.026069;
    const INV_RANGE: f32 = 1.0 / (MAX_EV - MIN_EV);

    let encode = |v: f32| -> f32 {
        let lv = v.max(1e-10).log2().clamp(MIN_EV, MAX_EV);
        let norm = (lv - MIN_EV) * INV_RANGE;
        agx_default_contrast_approx(norm)
    };

    let er = encode(r);
    let eg = encode(g);
    let eb = encode(b);

    // AgX outset matrix (returns ~gamma 2.2 display-referred values)
    let or = (1.19687900512017 * er - 0.0528968517574562 * eg - 0.0529716355144438 * eb)
        .clamp(0.0, 1.0);
    let og = (-0.0980208811401368 * er + 1.15190312990417 * eg - 0.0980434501171241 * eb)
        .clamp(0.0, 1.0);
    let ob = (-0.0990297440797205 * er - 0.0989611768448433 * eg + 1.15107367264116 * eb)
        .clamp(0.0, 1.0);

    // Linearize from AgX's ~2.2 curve so the final pipeline applies exact sRGB OETF uniformly.
    [or.powf(2.2), og.powf(2.2), ob.powf(2.2)]
}

#[inline]
fn rrt_and_odt_fit(v: [f32; 3]) -> [f32; 3] {
    let f = |x: f32| -> f32 {
        let a = x * (x + 0.0245786) - 0.000090537;
        let b = x * (0.983729 * x + 0.4329510) + 0.238081;
        a / b
    };
    [f(v[0]), f(v[1]), f(v[2])]
}

#[inline]
fn op_aces_fitted(c: [f32; 3]) -> [f32; 3] {
    // Stephen Hill ACES RRT + ODT fit (sRGB -> AP1-like -> fit -> sRGB)
    let r = 0.59719 * c[0] + 0.35458 * c[1] + 0.04823 * c[2];
    let g = 0.07600 * c[0] + 0.90834 * c[1] + 0.01566 * c[2];
    let b = 0.02840 * c[0] + 0.13383 * c[1] + 0.83777 * c[2];

    let m = rrt_and_odt_fit([r, g, b]);

    let or = 1.60475 * m[0] - 0.53108 * m[1] - 0.07367 * m[2];
    let og = -0.10208 * m[0] + 1.10813 * m[1] - 0.00605 * m[2];
    let ob = -0.00327 * m[0] - 0.07276 * m[1] + 1.07602 * m[2];

    [or.clamp(0.0, 1.0), og.clamp(0.0, 1.0), ob.clamp(0.0, 1.0)]
}

#[inline]
fn op_reinhard_luminance(c: [f32; 3], p: &PreparedParams) -> [f32; 3] {
    let y = luminance(c);
    let y_out = (y * (1.0 + y * p.reinhard_inv_lwhite_sq)) / (1.0 + y);
    finish_luminance_mapped(c, y, y_out.clamp(0.0, 1.0), p.params.highlight_desat)
}

#[inline]
fn op_windows_clip(c: [f32; 3]) -> [f32; 3] {
    [c[0].clamp(0.0, 1.0), c[1].clamp(0.0, 1.0), c[2].clamp(0.0, 1.0)]
}

#[inline]
fn tonemap_pixel_prepared(raw_scrgb: [f32; 3], p: &PreparedParams) -> [f32; 3] {
    let gamut_safe = map_negative_gamut(raw_scrgb, p.params.gamut_mapping);

    let norm = [
        gamut_safe[0] * p.factor,
        gamut_safe[1] * p.factor,
        gamut_safe[2] * p.factor,
    ];

    if p.params.show_hdr_heatmap {
        let peak_ch = norm[0].max(norm[1]).max(norm[2]);
        if peak_ch > 1.02 {
            // Color-coded excess above SDR white:
            // 1x..2x -> yellow, 2x..4x -> orange, 4x+ -> magenta/red
            let t = ((peak_ch - 1.0) / 4.0).clamp(0.0, 1.0);
            return if t < 0.5 {
                let k = t * 2.0;
                [1.0, 1.0 - 0.55 * k, 0.08 * (1.0 - k)]
            } else {
                let k = (t - 0.5) * 2.0;
                [1.0, 0.45 * (1.0 - k), 0.75 * k]
            };
        } else {
            // Dim monochrome SDR background so HDR highlights pop clearly
            let l = linear_to_srgb(luminance(norm)) * 0.45;
            return [l, l, l];
        }
    }

    let linear_sdr = match p.params.operator {
        TonemapOperator::HybridSdrSafe => op_hybrid_sdr_safe(norm, p),
        TonemapOperator::Bt2390 => op_bt2390(norm, p),
        TonemapOperator::PbrNeutral => op_pbr_neutral(norm, p),
        TonemapOperator::AgX => op_agx(norm),
        TonemapOperator::AcesFitted => op_aces_fitted(norm),
        TonemapOperator::ReinhardLuminance => op_reinhard_luminance(norm, p),
        TonemapOperator::WindowsClip => op_windows_clip(norm),
    };

    [
        linear_to_srgb(linear_sdr[0]),
        linear_to_srgb(linear_sdr[1]),
        linear_to_srgb(linear_sdr[2]),
    ]
}

/// Evaluates a single linear scRGB pixel into sRGB `[0, 1]` according to `params`.
#[inline]
#[allow(dead_code)]
pub fn tonemap_pixel_hdr(raw_scrgb: [f32; 3], params: &TonemapParams) -> [f32; 3] {
    let prepared = PreparedParams::new(*params);
    tonemap_pixel_prepared(raw_scrgb, &prepared)
}

/// Tonemaps an entire `CapturedFrame` into packed BGRA8 (`Vec<u8>`) suitable directly for
/// GPUI `RenderImage` and Win32 clipboard DIBs. Runs in parallel across CPU cores via Rayon.
pub fn tonemap_to_bgra8(frame: &CapturedFrame, params: &TonemapParams) -> Vec<u8> {
    tonemap_to_bgra8_split(frame, params, None)
}

/// Tonemaps an entire `CapturedFrame` into packed BGRA8 with an optional A/B vertical split
/// (`split_ratio` in `0.0..1.0`) where pixels to the right of the split use `WindowsClip`.
#[allow(dead_code)]
pub fn tonemap_to_bgra8_split(
    frame: &CapturedFrame,
    params: &TonemapParams,
    split_ratio: Option<f32>,
) -> Vec<u8> {
    let pixel_count = frame.rgb.len();
    let mut out = vec![0u8; pixel_count * 4];
    let width = frame.width.max(1) as usize;

    if frame.is_hdr {
        let prepared_left = PreparedParams::new(*params);
        if let Some(ratio) = split_ratio {
            let split_x = ((ratio.clamp(0.0, 1.0) * width as f32) as usize).min(width);
            let right_params = TonemapParams {
                operator: TonemapOperator::WindowsClip,
                show_hdr_heatmap: false,
                ..*params
            };
            let prepared_right = PreparedParams::new(right_params);

            out.par_chunks_exact_mut(width * 4)
                .zip(frame.rgb.par_chunks_exact(width))
                .for_each(|(dst_row, src_row)| {
                    for x in 0..width {
                        let prep = if x < split_x {
                            &prepared_left
                        } else {
                            &prepared_right
                        };
                        let srgb = tonemap_pixel_prepared(src_row[x], prep);
                        let base = x * 4;
                        dst_row[base] = float_to_u8(srgb[2]);
                        dst_row[base + 1] = float_to_u8(srgb[1]);
                        dst_row[base + 2] = float_to_u8(srgb[0]);
                        dst_row[base + 3] = 255;
                    }
                });
        } else {
            out.par_chunks_exact_mut(4)
                .zip(frame.rgb.par_iter())
                .for_each(|(dst, &src)| {
                    let srgb = tonemap_pixel_prepared(src, &prepared_left);
                    dst[0] = float_to_u8(srgb[2]); // B
                    dst[1] = float_to_u8(srgb[1]); // G
                    dst[2] = float_to_u8(srgb[0]); // R
                    dst[3] = 255; // A
                });
        }
    } else {
        // SDR frame is already sRGB-encoded [0, 1]; pass through unchanged.
        out.par_chunks_exact_mut(4)
            .zip(frame.rgb.par_iter())
            .for_each(|(dst, &src)| {
                dst[0] = float_to_u8(src[2]);
                dst[1] = float_to_u8(src[1]);
                dst[2] = float_to_u8(src[0]);
                dst[3] = 255;
            });
    }

    out
}

/// Samples the active operator's neutral luminance transfer curve from `0.0` to `max_x_sdr`
/// (in multiples of SDR white, e.g. `5.0`), returning normalized `(x_norm, y_linear_out)` pairs.
pub fn sample_tone_curve(params: &TonemapParams, max_x_sdr: f32, samples: usize) -> Vec<(f32, f32)> {
    let clean = TonemapParams {
        show_hdr_heatmap: false,
        ..*params
    };
    let prepared = PreparedParams::new(clean);
    let sdr_scale = (params.sdr_white_nits / 80.0).max(0.1);
    let count = samples.max(2);
    let mut pts = Vec::with_capacity(count + 1);

    for i in 0..=count {
        let xn = i as f32 / count as f32;
        let scrgb = xn * max_x_sdr * sdr_scale;
        let out = tonemap_pixel_prepared([scrgb, scrgb, scrgb], &prepared);
        pts.push((xn, out[0].clamp(0.0, 1.0)));
    }
    pts
}

/// Converts a packed BGRA8 buffer to RGB8 (for PNG encoding).
pub fn bgra8_to_rgb8(bgra: &[u8]) -> Vec<u8> {
    let pixel_count = bgra.len() / 4;
    let mut rgb = vec![0u8; pixel_count * 3];
    rgb.par_chunks_exact_mut(3)
        .zip(bgra.par_chunks_exact(4))
        .for_each(|(dst, src)| {
            dst[0] = src[2];
            dst[1] = src[1];
            dst[2] = src[0];
        });
    rgb
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hybrid_preserves_sdr_below_shoulder_exactly() {
        let params = TonemapParams {
            operator: TonemapOperator::HybridSdrSafe,
            sdr_white_nits: 240.0,
            exposure_ev: 0.0,
            max_luminance_nits: 1000.0,
            shoulder_start: 0.80,
            highlight_desat: 0.35,
            gamut_mapping: GamutMapping::SoftLuminance,
            show_hdr_heatmap: false,
        };

        // 0.5 linear SDR = 120 nits = 1.5 in scRGB
        let out = tonemap_pixel_hdr([1.5, 1.5, 1.5], &params);
        let expected = linear_to_srgb(0.5);
        assert!((out[0] - expected).abs() < 1e-5);
        assert!((out[1] - expected).abs() < 1e-5);
        assert!((out[2] - expected).abs() < 1e-5);
    }

    #[test]
    fn all_operators_are_monotonic_and_bounded_on_neutral_ramp() {
        for &op in TonemapOperator::ALL {
            let params = TonemapParams {
                operator: op,
                ..Default::default()
            };
            let mut prev = -1.0_f32;
            for i in 0..=100 {
                let scrgb = (i as f32 / 100.0).powf(2.0) * 50.0; // 0 to 4000 nits
                let out = tonemap_pixel_hdr([scrgb, scrgb, scrgb], &params);
                for ch in out {
                    assert!(ch.is_finite(), "{:?} produced non-finite value", op);
                    assert!((0.0..=1.0).contains(&ch), "{:?} out of bounds: {}", op, ch);
                }
                assert!(
                    out[0] + 1e-4 >= prev,
                    "{:?} not monotonic at {}: {} < {}",
                    op,
                    scrgb,
                    out[0],
                    prev
                );
                prev = out[0];
            }
        }
    }

    #[test]
    fn negative_gamut_mapping_preserves_luminance() {
        let c = [2.0, -0.3, -0.1];
        let y_orig = luminance(c);
        let mapped = map_negative_gamut(c, GamutMapping::SoftLuminance);
        assert!(mapped[0] >= 0.0 && mapped[1] >= 0.0 && mapped[2] >= 0.0);
        assert!((luminance(mapped) - y_orig).abs() < 1e-5);
    }
}
