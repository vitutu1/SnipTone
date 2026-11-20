use anyhow::{Context, Result};
use std::collections::HashMap;
use std::mem::size_of;
use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
    DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL, DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
    DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER,
    DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_SDR_WHITE_LEVEL, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
    DISPLAYCONFIG_TARGET_DEVICE_NAME, DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes,
    QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
};
use windows::Win32::Foundation::{ERROR_SUCCESS, POINT};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020, DXGI_MODE_ROTATION,
    DXGI_MODE_ROTATION_ROTATE90, DXGI_MODE_ROTATION_ROTATE180, DXGI_MODE_ROTATION_ROTATE270,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_ERROR_NOT_FOUND, IDXGIFactory1, IDXGIOutput6,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
use windows::core::Interface;

/// Usable desktop work area on a display (monitor bounds minus Windows Taskbar / docked appbars
/// on whichever edge — Left, Right, Top, or Bottom — it occupies).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkArea {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
}

impl WorkArea {
    pub fn right(self) -> i32 {
        self.left + self.width
    }

    pub fn bottom(self) -> i32 {
        self.top + self.height
    }

    /// Clamps a `(w, h)` window to stay strictly inside this work area with `margin` pixels
    /// of clearance from all 4 edges (monitor edge OR Windows Taskbar edge).
    pub fn clamp_window(self, x: i32, y: i32, w: i32, h: i32, margin: i32) -> (i32, i32) {
        let min_x = self.left + margin;
        let max_x = (self.right() - w - margin).max(min_x);
        let min_y = self.top + margin;
        let max_y = (self.bottom() - h - margin).max(min_y);
        (x.clamp(min_x, max_x), y.clamp(min_y, max_y))
    }
}

fn work_area_from_hmonitor(hmon: HMONITOR) -> Option<WorkArea> {
    if hmon.0.is_null() {
        return None;
    }
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if unsafe { GetMonitorInfoW(hmon, &mut info) }.as_bool() {
        let r = info.rcWork;
        let w = (r.right - r.left).max(1);
        let h = (r.bottom - r.top).max(1);
        Some(WorkArea {
            left: r.left,
            top: r.top,
            width: w,
            height: h,
        })
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayRotation {
    Identity,
    Rotate90,
    Rotate180,
    Rotate270,
}

impl DisplayRotation {
    pub fn from_dxgi(rot: DXGI_MODE_ROTATION) -> Self {
        if rot == DXGI_MODE_ROTATION_ROTATE90 {
            Self::Rotate90
        } else if rot == DXGI_MODE_ROTATION_ROTATE180 {
            Self::Rotate180
        } else if rot == DXGI_MODE_ROTATION_ROTATE270 {
            Self::Rotate270
        } else {
            Self::Identity
        }
    }

    pub fn degrees(self) -> u16 {
        match self {
            Self::Identity => 0,
            Self::Rotate90 => 90,
            Self::Rotate180 => 180,
            Self::Rotate270 => 270,
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct DisplayInfo {
    pub id: usize,
    pub adapter_index: u32,
    pub output_index: u32,
    pub gdi_name: String,
    pub friendly_name: String,
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    pub work_area: WorkArea,
    pub rotation: DisplayRotation,
    pub is_primary: bool,
    pub is_hdr: bool,
    pub sdr_white_nits: f32,
    pub max_luminance_nits: f32,
    pub min_luminance_nits: f32,
    pub bits_per_color: u32,
}

impl DisplayInfo {
    pub fn is_portrait(&self) -> bool {
        self.height > self.width
    }

    /// Queries the live usable work area (`rcWork`, excluding the Windows Taskbar on any edge)
    /// for this monitor, falling back to the cached `work_area` from display enumeration.
    pub fn live_work_area(&self) -> WorkArea {
        let center = POINT {
            x: self.left + (self.width as i32) / 2,
            y: self.top + (self.height as i32) / 2,
        };
        let hmon = unsafe { MonitorFromPoint(center, MONITOR_DEFAULTTONEAREST) };
        work_area_from_hmonitor(hmon).unwrap_or(self.work_area)
    }

    pub fn contains_point(&self, x: i32, y: i32) -> bool {
        x >= self.left
            && x < self.left + self.width as i32
            && y >= self.top
            && y < self.top + self.height as i32
    }

    #[allow(dead_code)]
    pub fn short_label(&self) -> String {
        let name = if self.friendly_name.is_empty() {
            self.gdi_name.trim_start_matches(r"\\.\").to_string()
        } else {
            self.friendly_name.clone()
        };
        let mode = if self.is_hdr {
            format!("{}b HDR · {:.0} nits", self.bits_per_color, self.sdr_white_nits)
        } else {
            format!("{}b SDR", self.bits_per_color)
        };
        format!("{} ({}x{} · {})", name, self.width, self.height, mode)
    }
}

#[derive(Debug, Default)]
struct DisplayConfigMeta {
    friendly_name: String,
    advanced_color_enabled: bool,
    sdr_white_nits: Option<f32>,
}

fn utf16_z_to_string(slice: &[u16]) -> String {
    let end = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
    String::from_utf16_lossy(&slice[..end])
}

fn query_display_config_map() -> HashMap<String, DisplayConfigMeta> {
    let mut map = HashMap::new();

    unsafe {
        let mut path_count = 0u32;
        let mut mode_count = 0u32;

        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
            != ERROR_SUCCESS
            || path_count == 0
        {
            return map;
        }

        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];

        if QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut path_count,
            paths.as_mut_ptr(),
            &mut mode_count,
            modes.as_mut_ptr(),
            None,
        ) != ERROR_SUCCESS
        {
            return map;
        }

        for path in paths.iter().take(path_count as usize) {
            let mut src_name = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                    size: size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                    adapterId: path.sourceInfo.adapterId,
                    id: path.sourceInfo.id,
                },
                ..Default::default()
            };

            if DisplayConfigGetDeviceInfo(&mut src_name.header) != ERROR_SUCCESS.0 as i32 {
                continue;
            }
            let gdi_name = utf16_z_to_string(&src_name.viewGdiDeviceName);
            if gdi_name.is_empty() {
                continue;
            }

            let mut meta = DisplayConfigMeta::default();

            let mut target_name = DISPLAYCONFIG_TARGET_DEVICE_NAME {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
                    size: size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32,
                    adapterId: path.targetInfo.adapterId,
                    id: path.targetInfo.id,
                },
                ..Default::default()
            };
            if DisplayConfigGetDeviceInfo(&mut target_name.header) == ERROR_SUCCESS.0 as i32 {
                meta.friendly_name = utf16_z_to_string(&target_name.monitorFriendlyDeviceName);
            }

            let mut adv_color = DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
                    size: size_of::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>() as u32,
                    adapterId: path.targetInfo.adapterId,
                    id: path.targetInfo.id,
                },
                ..Default::default()
            };
            if DisplayConfigGetDeviceInfo(&mut adv_color.header) == ERROR_SUCCESS.0 as i32 {
                // Bit 1 = advancedColorEnabled
                meta.advanced_color_enabled = (adv_color.Anonymous.value & 0x2) != 0;
            }

            let mut sdr_white = DISPLAYCONFIG_SDR_WHITE_LEVEL {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL,
                    size: size_of::<DISPLAYCONFIG_SDR_WHITE_LEVEL>() as u32,
                    adapterId: path.targetInfo.adapterId,
                    id: path.targetInfo.id,
                },
                ..Default::default()
            };
            if DisplayConfigGetDeviceInfo(&mut sdr_white.header) == ERROR_SUCCESS.0 as i32 {
                // Microsoft formula: SDRWhiteLevel / 1000.0 * 80.0 nits
                let nits = (sdr_white.SDRWhiteLevel as f32 / 1000.0) * 80.0;
                if nits.is_finite() && (40.0..=1200.0).contains(&nits) {
                    meta.sdr_white_nits = Some(nits);
                }
            }

            map.insert(gdi_name, meta);
        }
    }

    map
}

pub fn enumerate_displays() -> Result<Vec<DisplayInfo>> {
    let config_map = query_display_config_map();
    let factory: IDXGIFactory1 =
        unsafe { CreateDXGIFactory1() }.context("Failed to create IDXGIFactory1")?;

    let mut displays = Vec::new();
    let mut adapter_index = 0u32;

    loop {
        let adapter = match unsafe { factory.EnumAdapters1(adapter_index) } {
            Ok(a) => a,
            Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(_) => {
                adapter_index += 1;
                continue;
            }
        };

        let mut output_index = 0u32;
        loop {
            let output = match unsafe { adapter.EnumOutputs(output_index) } {
                Ok(o) => o,
                Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(_) => {
                    output_index += 1;
                    continue;
                }
            };

            if let Ok(desc) = unsafe { output.GetDesc() } {
                if desc.AttachedToDesktop.as_bool() {
                    let gdi_name = utf16_z_to_string(&desc.DeviceName);
                    let rect = desc.DesktopCoordinates;
                    let width = (rect.right - rect.left).max(0) as u32;
                    let height = (rect.bottom - rect.top).max(0) as u32;
                    let rotation = DisplayRotation::from_dxgi(desc.Rotation);
                    let is_primary = rect.left == 0 && rect.top == 0;
                    let work_area = work_area_from_hmonitor(desc.Monitor).unwrap_or(WorkArea {
                        left: rect.left,
                        top: rect.top,
                        width: width as i32,
                        height: height as i32,
                    });

                    let mut dxgi_is_hdr = false;
                    let mut max_lum = 1000.0_f32;
                    let mut min_lum = 0.0_f32;
                    let mut bits_per_color = 8u32;

                    if let Ok(out6) = output.cast::<IDXGIOutput6>() {
                        if let Ok(desc1) = unsafe { out6.GetDesc1() } {
                            dxgi_is_hdr =
                                desc1.ColorSpace == DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020;
                            if desc1.MaxLuminance > 80.0 && desc1.MaxLuminance.is_finite() {
                                max_lum = desc1.MaxLuminance;
                            }
                            if desc1.MinLuminance >= 0.0 && desc1.MinLuminance.is_finite() {
                                min_lum = desc1.MinLuminance;
                            }
                            if desc1.BitsPerColor > 0 {
                                bits_per_color = desc1.BitsPerColor;
                            }
                        }
                    }

                    let cfg = config_map.get(&gdi_name);
                    let is_hdr = cfg
                        .map(|m| m.advanced_color_enabled)
                        .unwrap_or(false)
                        || dxgi_is_hdr;
                    let sdr_white_nits = cfg.and_then(|m| m.sdr_white_nits).unwrap_or(240.0);
                    let friendly_name = cfg
                        .map(|m| m.friendly_name.clone())
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| format!("Monitor {}", displays.len() + 1));

                    displays.push(DisplayInfo {
                        id: displays.len(),
                        adapter_index,
                        output_index,
                        gdi_name,
                        friendly_name,
                        left: rect.left,
                        top: rect.top,
                        width,
                        height,
                        work_area,
                        rotation,
                        is_primary,
                        is_hdr,
                        sdr_white_nits,
                        max_luminance_nits: max_lum,
                        min_luminance_nits: min_lum,
                        bits_per_color,
                    });
                }
            }

            output_index += 1;
        }

        adapter_index += 1;
    }

    Ok(displays)
}

/// Returns the index of the display currently under the mouse cursor (works across mixed
/// landscape/portrait setups and negative virtual coordinates like `top = -481`).
pub fn display_index_at_cursor(displays: &[DisplayInfo]) -> Option<usize> {
    let mut pt = POINT::default();
    if unsafe { GetCursorPos(&mut pt) }.is_ok() {
        displays.iter().position(|d| d.contains_point(pt.x, pt.y))
    } else {
        None
    }
}
