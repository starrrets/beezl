//! Autostart and elevation.
//!
//! Autostart is the per-user Run key, so beezl shows up (and can be turned
//! off) in Task Manager's Startup apps. Installed copies also have an
//! on-demand task (see [`crate::task`]): the copy started from the Run key
//! is not elevated, so it starts the elevated one through the task, which
//! needs no UAC prompt, and exits. Turning autostart on or off never needs
//! admin rights.

use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

use crate::config::Config;
use crate::task::{task_xml, TASK_NAME};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_BINARY, RRF_RT_REG_SZ,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use super::{data_dir, wide};

const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const RUN_VALUE: PCWSTR = w!("beezl");
/// Run value of v0.1, when the app was called cuursor.
const LEGACY_RUN_VALUE: PCWSTR = w!("cuursor");
/// Where Task Manager records Startup apps the user turned off.
const APPROVED_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run");
/// Argument of the Run entry, so a logon start never opens a window.
pub const AUTOSTART_ARG: &str = "--autostart";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn schtasks(args: &[&str]) -> bool {
    Command::new("schtasks.exe")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn task_installed() -> bool {
    schtasks(&["/query", "/tn", TASK_NAME])
}

/// Create (or update) the task, and the Run entry unless autostart was
/// turned off. Needs admin; run by the installer.
pub fn register_task() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let xml = task_xml(&exe.to_string_lossy());
    // schtasks wants the XML file in UTF-16 with a BOM.
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(xml.encode_utf16().flat_map(|u| u.to_le_bytes()));
    let path = std::env::temp_dir().join("beezl-task.xml");
    if std::fs::write(&path, bytes).is_err() {
        return false;
    }
    let ok = schtasks(&["/create", "/tn", TASK_NAME, "/xml", &path.to_string_lossy(), "/f"]);
    let _ = std::fs::remove_file(path);
    if ok && super::load_config().autostart {
        set_run_key(true);
    }
    ok
}

pub fn unregister_task() {
    schtasks(&["/delete", "/tn", TASK_NAME, "/f"]);
    set_run_key(false);
}

fn launch_flag() -> PathBuf {
    data_dir().join("launch.flag")
}

/// Start the installed copy through its task, i.e. elevated. The flag tells
/// the started process that this was a manual launch, so it runs even if
/// autostart is off.
pub fn launch_via_task() -> bool {
    let flag = launch_flag();
    let _ = std::fs::create_dir_all(data_dir());
    let _ = std::fs::write(&flag, b"1");
    let ok = schtasks(&["/run", "/tn", TASK_NAME]);
    if !ok {
        let _ = std::fs::remove_file(flag);
    }
    ok
}

/// Consume the manual-launch flag left by [`launch_via_task`].
pub fn take_launch_flag() -> bool {
    let flag = launch_flag();
    let fresh = std::fs::metadata(&flag)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < Duration::from_secs(60));
    let _ = std::fs::remove_file(flag);
    fresh
}

/// Whether beezl starts with Windows: the Run entry exists and was not
/// turned off in Task Manager.
pub fn is_enabled(_cfg: &Config) -> bool {
    run_key().is_some() && !turned_off_in_task_manager()
}

/// Turn autostart on or off; the caller saves `cfg`.
pub fn set_enabled(cfg: &mut Config, on: bool) {
    cfg.autostart = on;
    set_run_key(on);
}

/// Restore a missing Run entry when autostart is on (for installs made by
/// older versions, or when the installer ran under another account).
pub fn ensure_run_key(cfg: &Config) {
    if cfg.autostart && run_key().is_none() {
        set_run_key(true);
    }
}

/// Task Manager stores a 12-byte value whose first byte is odd when the
/// entry is turned off.
fn turned_off_in_task_manager() -> bool {
    let mut buf = [0u8; 12];
    let mut len = buf.len() as u32;
    let err = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            APPROVED_KEY,
            RUN_VALUE,
            RRF_RT_REG_BINARY,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut len),
        )
    };
    err == ERROR_SUCCESS && len > 0 && buf[0] & 1 == 1
}

fn run_key() -> Option<()> {
    let err = unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE, RRF_RT_REG_SZ, None, None, None) };
    (err == ERROR_SUCCESS).then_some(())
}

/// Drop the per-user Run entries (uninstall, and the v0.1 cuursor entry).
pub fn remove_run_key() {
    set_run_key(false);
    unsafe {
        let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, LEGACY_RUN_VALUE);
    }
}

fn set_run_key(on: bool) {
    unsafe {
        // Either way, forget a "turned off" mark left by Task Manager.
        let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, APPROVED_KEY, RUN_VALUE);
        if !on {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE);
            return;
        }
        let Ok(exe) = std::env::current_exe() else { return };
        let data = wide(&format!("\"{}\" {AUTOSTART_ARG}", exe.display()));
        let _ = RegSetKeyValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            RUN_VALUE,
            REG_SZ.0,
            Some(data.as_ptr().cast()),
            (data.len() * 2) as u32,
        );
    }
}

pub fn is_elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elev = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elev as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && elev.TokenIsElevated != 0
    }
}
