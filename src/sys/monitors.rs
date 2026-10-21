//! Monitor enumeration: pixel rectangles from the window manager, physical
//! size from the EDID the driver stored in the registry.

use crate::config::Config;
use crate::edid;
use crate::layout::{Rect, Screen};
use windows::core::{BOOL, PCWSTR};
use windows::Win32::Foundation::{ERROR_SUCCESS, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayDevicesW, EnumDisplayMonitors, GetMonitorInfoW, DISPLAY_DEVICEW, HDC, HMONITOR, MONITORINFO,
    MONITORINFOEXW,
};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_BINARY};
use windows::Win32::UI::WindowsAndMessaging::{EDD_GET_DEVICE_INTERFACE_NAME, MONITORINFOF_PRIMARY};

use super::{from_wide, wide};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeSource {
    Config,
    Edid,
    Guess,
}

#[derive(Clone, Debug)]
pub struct Monitor {
    pub device: String,
    /// Stable per-monitor key (`GSM5B08\5&1a2b3c&0&UID4353`); tells apart two
    /// monitors of the same model. Falls back to the device name.
    pub key: String,
    pub model: String,
    pub rect: Rect,
    pub primary: bool,
    pub size_mm: (f64, f64),
    pub source: SizeSource,
}

impl Monitor {
    pub fn screen(&self) -> Screen {
        Screen { rect: self.rect, size_mm: self.size_mm }
    }

    pub fn diagonal_in(&self) -> f64 {
        self.size_mm.0.hypot(self.size_mm.1) / 25.4
    }

    /// Model name for people: EDID name, else a generic label.
    pub fn display_name(&self, index: usize) -> String {
        if self.model.is_empty() {
            format!("{} {}", crate::sys::tr("Монитор", "Monitor"), index + 1)
        } else {
            self.model.clone()
        }
    }
}

pub fn enumerate(cfg: &Config) -> Vec<Monitor> {
    let mut handles: Vec<HMONITOR> = Vec::new();
    unsafe extern "system" fn cb(h: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        unsafe { (*(data.0 as *mut Vec<HMONITOR>)).push(h) };
        BOOL(1)
    }
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(cb), LPARAM(&mut handles as *mut _ as isize));
    }

    let mut out: Vec<Monitor> = handles.into_iter().filter_map(|h| describe(h, cfg)).collect();
    // Primary first, then left to right; stable numbering for the info dialog.
    out.sort_by_key(|m| (!m.primary, m.rect.left, m.rect.top));
    out
}

fn describe(h: HMONITOR, cfg: &Config) -> Option<Monitor> {
    let mut mi = MONITORINFOEXW::default();
    mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    unsafe { GetMonitorInfoW(h, &mut mi as *mut _ as *mut MONITORINFO) }.ok().ok()?;
    let r = mi.monitorInfo.rcMonitor;
    let rect = Rect::new(r.left, r.top, r.right, r.bottom);
    let device = from_wide(&mi.szDevice);
    let primary = mi.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0;

    let (key, edid) = read_edid(&device);
    let key = if key.is_empty() { device.clone() } else { key };
    let model = edid.as_ref().map(|e| e.model.clone()).unwrap_or_default();
    let (w, h) = (rect.width(), rect.height());

    let over = cfg.monitors.iter().find(|o| o.matches(&key, &model, &device, w, h)).and_then(|o| o.size_mm(w, h));
    let (mut size_mm, source) = match (over, edid.and_then(|e| e.size_mm)) {
        (Some(s), _) => (s, SizeSource::Config),
        (None, Some(s)) => (s, SizeSource::Edid),
        // Unknown: assume ~96 DPI so it still links up sensibly.
        (None, None) => ((w as f64 * 0.2646, h as f64 * 0.2646), SizeSource::Guess),
    };
    // EDID describes the panel in landscape; follow the current rotation.
    if (w > h) != (size_mm.0 > size_mm.1) {
        size_mm = (size_mm.1, size_mm.0);
    }

    Some(Monitor { device, key, model, rect, primary, size_mm, source })
}

/// Monitor key (hardware id and instance) and parsed EDID of the monitor attached to an
/// adapter output such as `\\.\DISPLAY1`.
fn read_edid(device: &str) -> (String, Option<edid::Edid>) {
    let dev_w = wide(device);
    let mut dd = DISPLAY_DEVICEW { cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
    // The first monitor on that output; with mirroring there may be more.
    let found = unsafe { EnumDisplayDevicesW(PCWSTR(dev_w.as_ptr()), 0, &mut dd, EDD_GET_DEVICE_INTERFACE_NAME) };
    if !found.as_bool() {
        return (String::new(), None);
    }
    // \\?\DISPLAY#GSM5B08#5&1a2b3c&0&UID4353#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}
    let iface = from_wide(&dd.DeviceID);
    let parts: Vec<&str> = iface.trim_start_matches(r"\\?\").split('#').collect();
    if parts.len() < 3 {
        return (String::new(), None);
    }
    let id = format!(r"{}\{}", parts[1], parts[2]);
    let key = format!(r"SYSTEM\CurrentControlSet\Enum\{}\{}\{}\Device Parameters", parts[0], parts[1], parts[2]);
    (id, read_binary(&key, "EDID").as_deref().and_then(edid::parse))
}

fn read_binary(key: &str, value: &str) -> Option<Vec<u8>> {
    let (k, v) = (wide(key), wide(value));
    let mut buf = vec![0u8; 1024];
    let mut len = buf.len() as u32;
    let err = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(k.as_ptr()),
            PCWSTR(v.as_ptr()),
            RRF_RT_REG_BINARY,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut len),
        )
    };
    if err != ERROR_SUCCESS {
        return None;
    }
    buf.truncate(len as usize);
    Some(buf)
}
