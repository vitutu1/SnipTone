use crate::display::{DisplayInfo, DisplayRotation};
use crate::frame::{CapturedFrame, compute_frame_stats};
use anyhow::{Context, Result, anyhow};
use half::f16;
use rayon::prelude::*;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_FLAG,
    D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Resource,
    ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
    IDXGIAdapter, IDXGIDevice, IDXGIFactory1, IDXGIOutput1, IDXGIOutput5, IDXGIOutputDuplication,
    IDXGIResource,
};
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
    SetThreadDpiAwarenessContext,
};
use windows::core::{IInspectable, Interface};

/// Captures a single frame from the specified display.
/// Strategy:
/// 1. Try DXGI Desktop Duplication (`IDXGIOutput5::DuplicateOutput1` in `R16G16B16A16_FLOAT` when HDR).
/// 2. If DXGI returns `E_ACCESSDENIED` or fails (e.g. 4-client DWM limit, exclusive/protected state),
///    automatically fall back to Windows.Graphics.Capture (`R16G16B16A16Float` when HDR, `B8G8R8A8` when SDR).
pub fn capture_display(display: &DisplayInfo) -> Result<CapturedFrame> {
    capture_display_with_options(display, false)
}

pub fn capture_display_with_options(
    display: &DisplayInfo,
    capture_cursor: bool,
) -> Result<CapturedFrame> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let prev_dpi_ctx = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        let result = match capture_via_dxgi(display) {
            Ok(frame) => Ok(frame),
            Err(dxgi_err) => match capture_via_wgc(display, capture_cursor) {
                Ok(frame) => Ok(frame),
                Err(wgc_err) => Err(anyhow!(
                    "DXGI Desktop Duplication failed ({:#}) and WGC fallback also failed ({:#})",
                    dxgi_err,
                    wgc_err
                )),
            },
        };

        if !prev_dpi_ctx.is_invalid() {
            let _ = SetThreadDpiAwarenessContext(prev_dpi_ctx);
        }

        result
    }
}

unsafe fn readback_texture(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    src_tex: &ID3D11Texture2D,
    display: &DisplayInfo,
    backend_tag: &str,
) -> Result<CapturedFrame> {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { src_tex.GetDesc(&mut desc) };

    let stg_desc = D3D11_TEXTURE2D_DESC {
        Width: desc.Width,
        Height: desc.Height,
        MipLevels: 1,
        ArraySize: 1,
        Format: desc.Format,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
    };

    let mut stg_tex: Option<ID3D11Texture2D> = None;
    unsafe {
        device.CreateTexture2D(&stg_desc, None, Some(&mut stg_tex))?;
    }
    let stg_tex = stg_tex.ok_or_else(|| anyhow!("CreateTexture2D returned null"))?;

    let src_res: ID3D11Resource = src_tex.cast()?;
    let dst_res: ID3D11Resource = stg_tex.cast()?;
    unsafe {
        context.CopyResource(&dst_res, &src_res);
    }

    let width = desc.Width as usize;
    let height = desc.Height as usize;
    let format: DXGI_FORMAT = desc.Format;

    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe {
        context
            .Map(&dst_res, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
            .context("ID3D11DeviceContext::Map failed")?;
    }

    let row_pitch = mapped.RowPitch as usize;
    let total_bytes = row_pitch * height;
    let raw_bytes = unsafe {
        let slice = std::slice::from_raw_parts(mapped.pData as *const u8, total_bytes);
        let owned = slice.to_vec();
        context.Unmap(&dst_res, 0);
        owned
    };

    let is_hdr = format == DXGI_FORMAT_R16G16B16A16_FLOAT;
    let mut rgb = vec![[0.0f32; 3]; width * height];

    if is_hdr {
        rgb.par_chunks_mut(width)
            .enumerate()
            .for_each(|(y, out_row)| {
                let row_slice = &raw_bytes[y * row_pitch..y * row_pitch + width * 8];
                for x in 0..width {
                    let base = x * 8;
                    let r_bits = u16::from_le_bytes([row_slice[base], row_slice[base + 1]]);
                    let g_bits = u16::from_le_bytes([row_slice[base + 2], row_slice[base + 3]]);
                    let b_bits = u16::from_le_bytes([row_slice[base + 4], row_slice[base + 5]]);

                    let r = f16::from_bits(r_bits).to_f32();
                    let g = f16::from_bits(g_bits).to_f32();
                    let b = f16::from_bits(b_bits).to_f32();

                    out_row[x] = [
                        if r.is_finite() { r } else { 0.0 },
                        if g.is_finite() { g } else { 0.0 },
                        if b.is_finite() { b } else { 0.0 },
                    ];
                }
            });
    } else if format == DXGI_FORMAT_B8G8R8A8_UNORM {
        rgb.par_chunks_mut(width)
            .enumerate()
            .for_each(|(y, out_row)| {
                let row_slice = &raw_bytes[y * row_pitch..y * row_pitch + width * 4];
                for x in 0..width {
                    let base = x * 4;
                    let b = row_slice[base] as f32 / 255.0;
                    let g = row_slice[base + 1] as f32 / 255.0;
                    let r = row_slice[base + 2] as f32 / 255.0;
                    out_row[x] = [r, g, b];
                }
            });
    } else {
        return Err(anyhow!(
            "Unsupported DXGI capture format returned: {:?}",
            format
        ));
    }

    let stats = compute_frame_stats(&rgb, is_hdr, display.sdr_white_nits);
    let mode_tag = if is_hdr {
        format!("FP16 scRGB HDR · {}", backend_tag)
    } else {
        format!("BGRA8 SDR · {}", backend_tag)
    };

    let mut frame = CapturedFrame {
        width: desc.Width,
        height: desc.Height,
        is_hdr,
        sdr_white_nits: display.sdr_white_nits,
        monitor_max_nits: display.max_luminance_nits,
        source_label: format!("{} · {}", display.friendly_name, mode_tag),
        rgb,
        stats,
    };

    // If the capture backend returned an unrotated panel-native texture (e.g. 1920x1080 for a
    // 270°-rotated 1080x1920 portrait monitor), rotate it to match desktop orientation.
    if display.rotation != DisplayRotation::Identity
        && frame.width == display.height
        && frame.height == display.width
    {
        frame = frame.rotate(display.rotation);
    }

    Ok(frame)
}

unsafe fn capture_via_dxgi(display: &DisplayInfo) -> Result<CapturedFrame> {
    unsafe {
        let factory: IDXGIFactory1 =
            CreateDXGIFactory1().context("CreateDXGIFactory1 failed")?;
        let adapter = factory
            .EnumAdapters1(display.adapter_index)
            .with_context(|| format!("Adapter {} not found", display.adapter_index))?;
        let output = adapter
            .EnumOutputs(display.output_index)
            .with_context(|| format!("Output {} not found", display.output_index))?;

        let feature_levels = [
            D3D_FEATURE_LEVEL_11_0,
            D3D_FEATURE_LEVEL_10_1,
            D3D_FEATURE_LEVEL_10_0,
        ];
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;

        let adapter_base: IDXGIAdapter = adapter.cast()?;
        D3D11CreateDevice(
            Some(&adapter_base),
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_FLAG(0),
            Some(&feature_levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
        .context("D3D11CreateDevice failed")?;

        let device = device.ok_or_else(|| anyhow!("D3D11CreateDevice returned null device"))?;
        let context = context.ok_or_else(|| anyhow!("D3D11CreateDevice returned null context"))?;

        let output5: IDXGIOutput5 = output
            .cast()
            .context("IDXGIOutput5 not supported (requires Windows 10 1703+)")?;

        let duplication: IDXGIOutputDuplication = if display.is_hdr {
            let hdr_formats = [DXGI_FORMAT_R16G16B16A16_FLOAT];
            match output5.DuplicateOutput1(&device, 0, &hdr_formats) {
                Ok(dupl) => dupl,
                Err(fp16_err) => {
                    let sdr_formats = [DXGI_FORMAT_B8G8R8A8_UNORM];
                    match output5.DuplicateOutput1(&device, 0, &sdr_formats) {
                        Ok(dupl) => dupl,
                        Err(sdr_err) => {
                            let output1: IDXGIOutput1 = output.cast()?;
                            output1.DuplicateOutput(&device).with_context(|| {
                                format!(
                                    "DuplicateOutput1(FP16)={fp16_err}; DuplicateOutput1(BGRA8)={sdr_err}"
                                )
                            })?
                        }
                    }
                }
            }
        } else {
            let sdr_formats = [DXGI_FORMAT_B8G8R8A8_UNORM];
            match output5.DuplicateOutput1(&device, 0, &sdr_formats) {
                Ok(dupl) => dupl,
                Err(_) => {
                    let output1: IDXGIOutput1 = output.cast()?;
                    output1
                        .DuplicateOutput(&device)
                        .context("DuplicateOutput (BGRA8 SDR) failed")?
                }
            }
        };

        for attempt in 0..25 {
            let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource: Option<IDXGIResource> = None;

            match duplication.AcquireNextFrame(80, &mut frame_info, &mut resource) {
                Ok(()) => {
                    let res = resource.ok_or_else(|| anyhow!("AcquireNextFrame returned null"))?;
                    let src_tex: ID3D11Texture2D = res.cast()?;
                    let accept = frame_info.LastPresentTime != 0 || attempt >= 6;
                    if accept {
                        let frame_res =
                            readback_texture(&device, &context, &src_tex, display, "DXGI");
                        let _ = duplication.ReleaseFrame();
                        return frame_res;
                    } else {
                        let _ = duplication.ReleaseFrame();
                        thread::sleep(Duration::from_millis(16));
                    }
                }
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => {
                    thread::sleep(Duration::from_millis(20));
                    continue;
                }
                Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => {
                    return Err(anyhow!("DXGI_ERROR_ACCESS_LOST"));
                }
                Err(e) => return Err(e).context("AcquireNextFrame failed"),
            }
        }

        Err(anyhow!("Timed out waiting for a DXGI Desktop Duplication frame"))
    }
}

/// Fallback capture via Windows.Graphics.Capture (WGC) using `DirectXPixelFormat::R16G16B16A16Float`
/// for HDR monitors. Works even when DXGI Desktop Duplication returns `E_ACCESSDENIED` or hits the
/// 4-client DWM limit.
unsafe fn capture_via_wgc(display: &DisplayInfo, capture_cursor: bool) -> Result<CapturedFrame> {
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);

        let factory: IDXGIFactory1 =
            CreateDXGIFactory1().context("CreateDXGIFactory1 failed (WGC)")?;
        let adapter = factory.EnumAdapters1(display.adapter_index)?;
        let output = adapter.EnumOutputs(display.output_index)?;
        let hmonitor: HMONITOR = output.GetDesc()?.Monitor;

        let feature_levels = [
            D3D_FEATURE_LEVEL_11_0,
            D3D_FEATURE_LEVEL_10_1,
            D3D_FEATURE_LEVEL_10_0,
        ];
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        let adapter_base: IDXGIAdapter = adapter.cast()?;
        D3D11CreateDevice(
            Some(&adapter_base),
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&feature_levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
        .context("D3D11CreateDevice failed (WGC)")?;

        let device = device.ok_or_else(|| anyhow!("D3D11CreateDevice returned null (WGC)"))?;
        let context = context.ok_or_else(|| anyhow!("D3D11CreateDevice returned null ctx (WGC)"))?;

        let dxgi_device: IDXGIDevice = device.cast()?;
        let inspectable: IInspectable = CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device)
            .context("CreateDirect3D11DeviceFromDXGIDevice failed")?;
        let d3d_device: IDirect3DDevice = inspectable.cast()?;

        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .context("Failed to get IGraphicsCaptureItemInterop")?;
        let item: GraphicsCaptureItem = interop
            .CreateForMonitor(hmonitor)
            .context("CreateForMonitor failed")?;
        let size = item.Size().context("GraphicsCaptureItem::Size failed")?;

        let pixel_format = if display.is_hdr {
            DirectXPixelFormat::R16G16B16A16Float
        } else {
            DirectXPixelFormat::B8G8R8A8UIntNormalized
        };

        let frame_pool =
            Direct3D11CaptureFramePool::CreateFreeThreaded(&d3d_device, pixel_format, 1, size)
                .context("Direct3D11CaptureFramePool::CreateFreeThreaded failed")?;
        let session = frame_pool
            .CreateCaptureSession(&item)
            .context("CreateCaptureSession failed")?;
        let _ = session.SetIsBorderRequired(false);
        let _ = session.SetIsCursorCaptureEnabled(capture_cursor);

        let signal = Arc::new((Mutex::new(false), Condvar::new()));
        let signal_cb = Arc::clone(&signal);

        let token = frame_pool.FrameArrived(&TypedEventHandler::<
            Direct3D11CaptureFramePool,
            IInspectable,
        >::new(move |_, _| {
            let (lock, cvar) = &*signal_cb;
            if let Ok(mut ready) = lock.lock() {
                *ready = true;
                cvar.notify_all();
            }
            Ok(())
        }))?;

        session
            .StartCapture()
            .context("GraphicsCaptureSession::StartCapture failed")?;

        let (lock, cvar) = &*signal;
        let arrived = {
            let guard = lock.lock().map_err(|_| anyhow!("WGC signal mutex poisoned"))?;
            let (guard, _) = cvar
                .wait_timeout_while(guard, Duration::from_secs(3), |ready| !*ready)
                .map_err(|_| anyhow!("WGC wait failed"))?;
            *guard
        };

        let capture_result = if arrived {
            let frame = frame_pool
                .TryGetNextFrame()
                .context("WGC TryGetNextFrame failed")?;
            let surface = frame.Surface().context("WGC frame.Surface failed")?;
            let access: IDirect3DDxgiInterfaceAccess = surface.cast()?;
            let src_tex: ID3D11Texture2D = access
                .GetInterface()
                .context("WGC GetInterface::<ID3D11Texture2D> failed")?;
            let res = readback_texture(&device, &context, &src_tex, display, "WGC");
            let _ = frame.Close();
            res
        } else {
            Err(anyhow!("Timed out (3s) waiting for a WGC frame"))
        };

        let _ = frame_pool.RemoveFrameArrived(token);
        let _ = session.Close();
        let _ = frame_pool.Close();

        capture_result
    }
}
