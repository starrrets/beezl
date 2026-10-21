//! Tray icon drawn at runtime (two monitors of different size), so the build
//! needs no resource compiler.

use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, GetSystemMetrics, LoadIconW, HICON, ICONINFO, IDI_APPLICATION, SM_CXSMICON,
};

pub fn make(active: bool) -> HICON {
    let n = unsafe { GetSystemMetrics(SM_CXSMICON) }.clamp(16, 64);
    let px = draw(n, active);
    build(n, &px).unwrap_or_else(|| unsafe { LoadIconW(None, IDI_APPLICATION).unwrap_or_default() })
}

/// Premultiplied BGRA, top-down.
fn draw(n: i32, active: bool) -> Vec<u32> {
    let (frame, small, big) =
        if active { (0xFF_1E1E1E, 0xFF_4CA3FF, 0xFF_1F6FEB) } else { (0xFF_3A3A3A, 0xFF_A0A0A0, 0xFF_7A7A7A) };
    let mut px = vec![0u32; (n * n) as usize];
    // Design grid is 16x16, scaled to the system small-icon size.
    let mut rect = |x0: i32, y0: i32, x1: i32, y1: i32, c: u32| {
        let s = |v: i32| v * n / 16;
        for y in s(y0)..s(y1) {
            for x in s(x0)..s(x1) {
                px[(y * n + x) as usize] = c;
            }
        }
    };
    // Smaller monitor on the left, bottoms aligned, like the target setup.
    rect(0, 5, 7, 12, frame);
    rect(1, 6, 6, 11, small);
    rect(6, 2, 16, 12, frame);
    rect(7, 3, 15, 11, big);
    // Stands.
    rect(3, 12, 4, 14, frame);
    rect(10, 12, 12, 14, frame);
    rect(2, 14, 14, 15, frame);
    px
}

fn build(n: i32, px: &[u32]) -> Option<HICON> {
    unsafe {
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: n,
                biHeight: -n,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let color = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        std::ptr::copy_nonoverlapping(px.as_ptr(), bits.cast::<u32>(), px.len());
        // All-zero mask: the alpha channel decides transparency.
        let mask_bits = vec![0u8; ((n + 15) / 16 * 2 * n) as usize];
        let mask = CreateBitmap(n, n, 1, 1, Some(mask_bits.as_ptr().cast()));
        let info = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info).ok();
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}
