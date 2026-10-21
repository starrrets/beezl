//! Install and uninstall. The setup program is `cuursor-ui.exe` itself with
//! the background process (`cuursor.exe`) embedded, so one download carries
//! everything and the installed UI is just a copy of it. The windows are
//! the same Fluent UI as the app; the actual work runs in an elevated copy
//! of this process (`--do-install` / `--do-uninstall`).

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use cuursor::sys::{self, autostart, wide, DAEMON_CLASS};
use windows::core::{w, Interface, HSTRING, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, LPARAM, WAIT_OBJECT_0, WPARAM};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Registry::{
    RegCreateKeyExW, RegDeleteTreeW, RegGetValueW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_WRITE, REG_DWORD,
    REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ,
};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, GetExitCodeProcess, OpenProcess, TerminateProcess, WaitForSingleObject, INFINITE,
    PROCESS_TERMINATE,
};
use windows::Win32::UI::Shell::{
    IShellLinkW, ShellExecuteExW, ShellLink, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW, SW_HIDE, WM_CLOSE};

const UNINSTALL_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\cuursor";
/// Entry of the Inno Setup based v0.2 installer, replaced by this one.
const INNO_KEY: &str =
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\{6F1C2B7E-3D4A-4C8B-9E21-7A5D0C3B9F14}_is1";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// `cuursor.exe`, deflate-compressed, when built as the setup program.
static PAYLOAD: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/daemon.deflate"));

pub fn has_payload() -> bool {
    !PAYLOAD.is_empty()
}

pub fn install_dir() -> PathBuf {
    let pf = std::env::var_os("ProgramW6432")
        .or_else(|| std::env::var_os("ProgramFiles"))
        .unwrap_or_else(|| r"C:\Program Files".into());
    PathBuf::from(pf).join("cuursor")
}

/// True when this exe is the installed copy, not a downloaded setup file.
pub fn running_installed() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let same = |a: &Path, b: &Path| a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy());
    exe.parent().is_some_and(|d| same(d, &install_dir()))
}

pub fn installed_version() -> Option<String> {
    reg_string(UNINSTALL_KEY, "DisplayVersion").or_else(|| reg_string(INNO_KEY, "DisplayVersion"))
}

fn reg_string(key: &str, value: &str) -> Option<String> {
    let (k, v) = (wide(key), wide(value));
    let mut buf = [0u16; 260];
    let mut len = std::mem::size_of_val(&buf) as u32;
    let err = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(k.as_ptr()),
            PCWSTR(v.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut len),
        )
    };
    (err == ERROR_SUCCESS).then(|| sys::from_wide(&buf))
}

/// Run this exe elevated with `args` and wait. `Err` carries a message for
/// the user (including a declined UAC prompt).
pub fn run_elevated(args: &str) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let file = HSTRING::from(exe.as_os_str());
    let params = HSTRING::from(args);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe {
        ShellExecuteExW(&mut info).map_err(|_| {
            sys::tr("Нужно разрешение администратора.", "Administrator permission is required.").to_string()
        })?;
        let _ = WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = 1u32;
        let _ = GetExitCodeProcess(info.hProcess, &mut code);
        let _ = CloseHandle(info.hProcess);
        match code {
            0 => Ok(()),
            _ => Err(match std::fs::read_to_string(error_file()) {
                Ok(t) if !t.is_empty() => t,
                _ => format!("{} {code}", sys::tr("Код ошибки", "Error code")),
            }),
        }
    }
}

/// Where the elevated worker leaves an error message for the window.
fn error_file() -> PathBuf {
    std::env::temp_dir().join("cuursor-setup-error.txt")
}

/// Entry point of the elevated worker; returns the process exit code.
pub fn worker(install: bool) -> i32 {
    let _ = std::fs::remove_file(error_file());
    let res = if install { do_install() } else { do_uninstall() };
    match res {
        Ok(()) => 0,
        Err(e) => {
            let _ = std::fs::write(error_file(), e);
            1
        }
    }
}

fn do_install() -> Result<(), String> {
    let dir = install_dir();
    stop_running();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;

    // Background process from the embedded payload, UI as a copy of ourselves.
    let daemon = miniz_oxide::inflate::decompress_to_vec(PAYLOAD).map_err(|_| "payload".to_string())?;
    write_file(&dir.join("cuursor.exe"), &daemon)?;
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let ui = dir.join("cuursor-ui.exe");
    if !me.to_string_lossy().eq_ignore_ascii_case(&ui.to_string_lossy()) {
        let bytes = std::fs::read(&me).map_err(|e| e.to_string())?;
        write_file(&ui, &bytes)?;
    }

    // Leftovers of older versions: Inno Setup files and entry, v0.1 portable copy.
    for f in ["unins000.exe", "unins000.dat", "LICENSE"] {
        let _ = std::fs::remove_file(dir.join(f));
    }
    delete_key(INNO_KEY);
    autostart::remove_run_key();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        remove_file_retry(&PathBuf::from(local).join("cuursor").join("cuursor.exe"));
    }

    shortcut(&dir.join("cuursor.exe"))?;
    register_uninstall(&dir)?;

    let ok = Command::new(dir.join("cuursor.exe"))
        .arg("--register")
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        return Err(sys::tr("Не удалось настроить автозапуск.", "Could not set up autostart.").into());
    }
    Ok(())
}

fn do_uninstall() -> Result<(), String> {
    let dir = install_dir();
    stop_running();
    let _ = Command::new(dir.join("cuursor.exe")).arg("--unregister").creation_flags(CREATE_NO_WINDOW).status();
    let _ = std::fs::remove_file(start_menu_link());
    delete_key(UNINSTALL_KEY);
    delete_key(INNO_KEY);
    let _ = std::fs::remove_dir_all(sys::data_dir());
    let _ = std::fs::remove_file(dir.join("cuursor.exe"));
    // This exe may be the one in `dir`; let cmd remove the folder after we exit.
    let _ = Command::new("cmd.exe")
        .args(["/c", "ping", "-n", "3", "127.0.0.1", ">nul", "&", "rmdir", "/s", "/q"])
        .arg(&dir)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
    Ok(())
}

fn remove_file_retry(path: &Path) {
    let start = Instant::now();
    while path.exists() && std::fs::remove_file(path).is_err() && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    // A just-terminated process can hold its image for a moment.
    let start = Instant::now();
    loop {
        match std::fs::write(path, bytes) {
            Ok(()) => return Ok(()),
            Err(e) if start.elapsed() > Duration::from_secs(5) => return Err(format!("{}: {e}", path.display())),
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    }
}

/// Close the running app (politely first, so the tray icon goes away) and
/// any other cuursor process except ourselves and our parent.
fn stop_running() {
    unsafe {
        if let Ok(h) = FindWindowW(DAEMON_CLASS, None) {
            let _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0));
            let start = Instant::now();
            while FindWindowW(DAEMON_CLASS, None).is_ok() && start.elapsed() < Duration::from_secs(2) {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
    let me = unsafe { GetCurrentProcessId() };
    let parent = std::env::args().skip_while(|a| a != "--parent").nth(1).and_then(|p| p.parse::<u32>().ok());
    for (pid, name) in processes() {
        let n = name.to_ascii_lowercase();
        if (n == "cuursor.exe" || n == "cuursor-ui.exe") && pid != me && Some(pid) != parent {
            unsafe {
                if let Ok(h) = OpenProcess(PROCESS_TERMINATE, false, pid) {
                    let _ = TerminateProcess(h, 0);
                    let _ = WaitForSingleObject(h, 2000) == WAIT_OBJECT_0;
                    let _ = CloseHandle(h);
                }
            }
        }
    }
}

fn processes() -> Vec<(u32, String)> {
    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            out.push((e.th32ProcessID, sys::from_wide(&e.szExeFile)));
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    out
}

fn start_menu_link() -> PathBuf {
    let pd = std::env::var_os("ProgramData").unwrap_or_else(|| r"C:\ProgramData".into());
    PathBuf::from(pd).join(r"Microsoft\Windows\Start Menu\Programs\cuursor.lnk")
}

fn shortcut(target: &Path) -> Result<(), String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
        let t = HSTRING::from(target.as_os_str());
        link.SetPath(&t).map_err(|e| e.to_string())?;
        if let Some(d) = target.parent() {
            let _ = link.SetWorkingDirectory(&HSTRING::from(d.as_os_str()));
        }
        let _ = link.SetDescription(&HSTRING::from(sys::tr(
            "Плавный переход курсора между мониторами",
            "Seamless cursor movement between monitors",
        )));
        let file: IPersistFile = link.cast().map_err(|e| e.to_string())?;
        file.Save(&HSTRING::from(start_menu_link().as_os_str()), true).map_err(|e| e.to_string())
    }
}

fn register_uninstall(dir: &Path) -> Result<(), String> {
    let ui = dir.join("cuursor-ui.exe");
    let size_kb = ["cuursor.exe", "cuursor-ui.exe"]
        .iter()
        .filter_map(|f| std::fs::metadata(dir.join(f)).ok())
        .map(|m| m.len() / 1024)
        .sum::<u64>() as u32;
    unsafe {
        let mut key = HKEY::default();
        let k = wide(UNINSTALL_KEY);
        let err = RegCreateKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(k.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        );
        if err != ERROR_SUCCESS {
            return Err(format!("registry: {}", err.0));
        }
        let set_str = |name: &str, value: &str| {
            let n = wide(name);
            let v = wide(value);
            let bytes = std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), v.len() * 2);
            let _ = RegSetValueExW(key, PCWSTR(n.as_ptr()), None, REG_SZ, Some(bytes));
        };
        let set_dword = |name: &str, value: u32| {
            let n = wide(name);
            let _ = RegSetValueExW(key, PCWSTR(n.as_ptr()), None, REG_DWORD, Some(&value.to_le_bytes()));
        };
        set_str("DisplayName", "cuursor");
        set_str("DisplayVersion", env!("CARGO_PKG_VERSION"));
        set_str("Publisher", "Eugene Staroselets");
        set_str("URLInfoAbout", "https://github.com/starrrets/cuursor");
        set_str("InstallLocation", &dir.to_string_lossy());
        set_str("DisplayIcon", &format!("{},0", dir.join("cuursor.exe").display()));
        set_str("UninstallString", &format!("\"{}\" --uninstall", ui.display()));
        set_dword("NoModify", 1);
        set_dword("NoRepair", 1);
        set_dword("EstimatedSize", size_kb);
        let _ = windows::Win32::System::Registry::RegCloseKey(key);
    }
    Ok(())
}

fn delete_key(path: &str) {
    let k = wide(path);
    unsafe {
        let _ = RegDeleteTreeW(HKEY_LOCAL_MACHINE, PCWSTR(k.as_ptr()));
    }
}
