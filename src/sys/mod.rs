//! Windows plumbing shared by the background process (`beezl.exe`) and the
//! UI (`beezl-ui.exe`): paths, settings file, monitors, autostart and the
//! tiny message protocol between the two.

pub mod autostart;
pub mod monitors;

use std::path::PathBuf;

use crate::config::{self, Config};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::Globalization::GetUserDefaultUILanguage;
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW, WM_APP};

/// Class of the background process's hidden window; also how others find it.
pub const DAEMON_CLASS: PCWSTR = w!("beezl.tray");
/// UI → background: settings changed on disk, re-read them.
pub const WM_BEEZL_RELOAD: u32 = WM_APP + 20;
/// Second launch → background: show the settings window.
pub const WM_BEEZL_SHOW_SETTINGS: u32 = WM_APP + 21;

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("beezl")
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.toml")
}

/// Settings from disk; anything missing or unreadable falls back to defaults.
pub fn load_config() -> Config {
    std::fs::read_to_string(config_path()).ok().and_then(|t| config::parse(&t).ok()).unwrap_or_default()
}

pub fn save_config(cfg: &Config) -> std::io::Result<()> {
    let path = config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Write-then-rename so the background process never reads half a file.
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, cfg.to_toml())?;
    std::fs::rename(tmp, path)
}

/// Post a message to the running background process, if any.
pub fn notify_daemon(msg: u32) -> bool {
    unsafe {
        match FindWindowW(DAEMON_CLASS, None) {
            Ok(hwnd) => PostMessageW(Some(hwnd), msg, WPARAM(0), LPARAM(0)).is_ok(),
            Err(_) => false,
        }
    }
}

/// UI language: Russian when Windows is Russian, English otherwise.
pub fn is_russian() -> bool {
    (unsafe { GetUserDefaultUILanguage() } & 0x3FF) == 0x19
}

pub fn tr(ru: &'static str, en: &'static str) -> &'static str {
    if is_russian() {
        ru
    } else {
        en
    }
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}
