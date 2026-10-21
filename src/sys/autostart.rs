//! Autostart and elevation.
//!
//! Installed copies use a logon task (see [`crate::task`]) that starts
//! beezl elevated without a UAC prompt. Portable copies fall back to the
//! per-user Run key. Whether autostart is wanted is stored in the settings
//! file, so turning it off never needs admin rights: the task still fires at
//! logon and the app exits straight away.

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
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use super::{data_dir, wide};

const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const RUN_VALUE: PCWSTR = w!("beezl");
/// Run value of v0.1, when the app was called cuursor.
const LEGACY_RUN_VALUE: PCWSTR = w!("cuursor");
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

/// Create (or update) the logon task. Needs admin; run by the installer.
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
    if ok {
        // The task replaces the per-user Run entry of portable/older versions.
        set_run_key(false);
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

pub fn is_enabled(cfg: &Config) -> bool {
    if task_installed() {
        cfg.autostart
    } else {
        run_key().is_some()
    }
}

/// Turn autostart on or off; the caller saves `cfg`.
pub fn set_enabled(cfg: &mut Config, on: bool) {
    cfg.autostart = on;
    if !task_installed() {
        set_run_key(on);
    }
}

fn run_key() -> Option<()> {
    let err = unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE, RRF_RT_REG_SZ, None, None, None) };
    (err == ERROR_SUCCESS).then_some(())
}

/// Drop the per-user Run entry (portable and v0.1 copies); the logon task
/// replaces it.
pub fn remove_run_key() {
    set_run_key(false);
    unsafe {
        let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, LEGACY_RUN_VALUE);
    }
}

fn set_run_key(on: bool) {
    unsafe {
        if !on {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE);
            return;
        }
        let Ok(exe) = std::env::current_exe() else { return };
        let data = wide(&format!("\"{}\"", exe.display()));
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
