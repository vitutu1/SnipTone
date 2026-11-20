use anyhow::{Context, Result, anyhow};
use std::mem::size_of;
use std::ptr::copy_nonoverlapping;
use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND};
use windows::Win32::Graphics::Gdi::{BI_RGB, BITMAPINFOHEADER};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_DIB;

/// Copies a top-down BGRA8 image to the Windows clipboard as a 32-bit `CF_DIB` bitmap.
/// Compatible with Discord, Slack, Chrome, Paint, Telegram, etc.
pub fn copy_bgra8_to_clipboard(width: u32, height: u32, bgra: &[u8]) -> Result<()> {
    let expected = (width as usize) * (height as usize) * 4;
    if bgra.len() != expected {
        return Err(anyhow!(
            "Clipboard buffer size mismatch: got {}, expected {}",
            bgra.len(),
            expected
        ));
    }

    let header_size = size_of::<BITMAPINFOHEADER>();
    let total_size = header_size + expected;

    unsafe {
        let hglobal: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, total_size)
            .context("GlobalAlloc failed for clipboard DIB")?;

        let ptr = GlobalLock(hglobal) as *mut u8;
        if ptr.is_null() {
            return Err(anyhow!("GlobalLock returned null"));
        }

        let header = BITMAPINFOHEADER {
            biSize: header_size as u32,
            biWidth: width as i32,
            // Negative height = top-down DIB (matches our buffer directly, no vertical flip needed,
            // though some legacy apps prefer bottom-up so we flip into bottom-up positive height!)
            biHeight: height as i32,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: expected as u32,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        };

        copy_nonoverlapping(&header as *const _ as *const u8, ptr, header_size);

        // Write bottom-up rows for maximum compatibility with Windows applications
        let pixels_dst = ptr.add(header_size);
        let row_bytes = (width as usize) * 4;
        for y in 0..(height as usize) {
            let src_row = (height as usize - 1 - y) * row_bytes;
            copy_nonoverlapping(
                bgra.as_ptr().add(src_row),
                pixels_dst.add(y * row_bytes),
                row_bytes,
            );
        }

        let _ = GlobalUnlock(hglobal);

        OpenClipboard(Some(HWND::default())).context("OpenClipboard failed")?;
        let res = (|| -> Result<()> {
            EmptyClipboard().context("EmptyClipboard failed")?;
            SetClipboardData(CF_DIB.0 as u32, Some(HANDLE(hglobal.0)))
                .context("SetClipboardData(CF_DIB) failed")?;
            Ok(())
        })();
        let _ = CloseClipboard();
        res
    }
}
