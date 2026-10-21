//! Background process: owns the mouse hook and the tray icon. All windows
//! (tray flyout, settings) live in `cuursor-ui.exe`, started on demand, so
//! this process stays small and nothing UI-related can delay mouse input.

mod hook;

use std::cell::RefCell;
use std::os::windows::io::AsRawHandle;
use std::process::{Child, Command};
use std::sync::atomic::Ordering;

use cuursor::config::Config;
use cuursor::layout::Layout;
use cuursor::sys::{self, autostart, monitors, tr, wide, DAEMON_CLASS, WM_CUURSOR_RELOAD, WM_CUURSOR_SHOW_SETTINGS};
use cuursor::task::FROM_TASK_ARG;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, FILETIME, HANDLE, HWND, LPARAM, LRESULT, POINT, RECT,
    WPARAM,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
use windows::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcess, GetProcessTimes, SetPriorityClass, ABOVE_NORMAL_PRIORITY_CLASS,
};
use windows::Win32::UI::Controls::{LoadIconMetric, LIM_SMALL};
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconGetRect, Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW, NOTIFYICONIDENTIFIER, NOTIFY_ICON_MESSAGE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, AppendMenuW, ChangeWindowMessageFilterEx, CreatePopupMenu, CreateWindowExW,
    DefWindowProcW, DestroyIcon, DestroyMenu, DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW, KillTimer,
    MessageBoxW, PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SetForegroundWindow, SetTimer,
    TrackPopupMenu, TranslateMessage, ASFW_ANY, HICON, MB_ICONERROR, MB_OK, MESSAGEBOX_STYLE, MF_SEPARATOR, MF_STRING,
    MSG, MSGFLT_ALLOW, TPM_RIGHTBUTTON, WINDOW_EX_STYLE, WM_APP, WM_COMMAND, WM_DESTROY, WM_DISPLAYCHANGE,
    WM_DPICHANGED, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WM_SETTINGCHANGE, WM_TIMER, WNDCLASSW, WS_OVERLAPPED,
};

const WM_TRAY: u32 = WM_APP + 10;
const TIMER_RELOAD: usize = 1;

const ID_SETTINGS: usize = 1;
const ID_EXIT: usize = 2;

/// A click on the tray icon that closed the flyout must not reopen it.
const REOPEN_GUARD_MS: u64 = 400;

struct App {
    hwnd: HWND,
    cfg: Config,
    icon_on: HICON,
    icon_off: HICON,
    taskbar_created: u32,
    flyout: Option<Child>,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

pub fn run() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |a: &str| args.iter().any(|x| x == a);

    // Installer hooks.
    if has("--register") {
        std::process::exit(if autostart::register_task() { 0 } else { 1 });
    }
    if has("--unregister") {
        autostart::unregister_task();
        return;
    }

    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let from_task = has(FROM_TASK_ARG);

    // Already running: a manual launch just opens the settings.
    if unsafe { windows::Win32::UI::WindowsAndMessaging::FindWindowW(DAEMON_CLASS, None) }.is_ok() {
        if !from_task {
            // Let the running instance bring its window to the front.
            unsafe {
                let _ = AllowSetForegroundWindow(ASFW_ANY);
            }
            sys::notify_daemon(WM_CUURSOR_SHOW_SETTINGS);
        }
        return;
    }
    // Prefer the elevated instance started by the installed logon task, so the
    // hook also works over admin windows.
    if !from_task && !autostart::is_elevated() && autostart::launch_via_task() {
        return;
    }
    unsafe {
        let _ = CreateMutexW(None, true, w!("Local\\cuursor-single-instance"));
        let e = GetLastError();
        if e == ERROR_ALREADY_EXISTS || e == ERROR_ACCESS_DENIED {
            return;
        }
    }
    let manual = autostart::take_launch_flag();
    let first_run = !sys::config_path().exists();
    let cfg = sys::load_config();
    if from_task && !manual && !cfg.autostart {
        return;
    }
    if first_run {
        let _ = sys::save_config(&cfg);
    }

    unsafe {
        let _ = SetPriorityClass(GetCurrentProcess(), ABOVE_NORMAL_PRIORITY_CLASS);
    }
    hook::ENABLED.store(cfg.enabled, Ordering::Relaxed);
    hook::start(params(&cfg));

    let hwnd = create_window();
    let (icon_on, icon_off) = load_icons();
    APP.with(|a| {
        *a.borrow_mut() = Some(App {
            hwnd,
            cfg,
            icon_on,
            icon_off,
            taskbar_created: unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) },
            flyout: None,
        })
    });
    tray(NIM_ADD);
    if first_run {
        spawn_ui(&["--settings"]);
    }

    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    tray(NIM_DELETE);
    hook::stop();
}

fn params(cfg: &Config) -> hook::Params {
    let mons = monitors::enumerate(cfg);
    hook::Params {
        layout: Layout::build(mons.iter().map(|m| m.screen()).collect(), cfg.mode, cfg.align),
        skip_injected: cfg.skip_injected,
        max_jump_px: cfg.max_jump_px.max(1),
    }
}

/// Re-read settings and monitors and hand a fresh layout to the hook.
fn reload() {
    let cfg = sys::load_config();
    hook::ENABLED.store(cfg.enabled, Ordering::Relaxed);
    hook::update(params(&cfg));
    APP.with(|a| {
        if let Some(app) = a.borrow_mut().as_mut() {
            app.cfg = cfg;
        }
    });
    tray(NIM_MODIFY);
}

fn load_icons() -> (HICON, HICON) {
    unsafe {
        let inst = GetModuleHandleW(None).unwrap_or_default();
        let load =
            |id: usize| LoadIconMetric(Some(inst.into()), PCWSTR(id as *const u16), LIM_SMALL).unwrap_or_default();
        (load(1), load(2))
    }
}

fn create_window() -> HWND {
    unsafe {
        let instance = GetModuleHandleW(None).expect("module handle");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: DAEMON_CLASS,
            ..Default::default()
        };
        RegisterClassW(&wc);
        // A real (hidden) top-level window: message-only windows miss WM_DISPLAYCHANGE.
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            DAEMON_CLASS,
            w!("cuursor"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .unwrap_or_else(|_| fatal(tr("Не удалось создать окно.", "Could not create the window.")));
        // When elevated, let the non-elevated UI and launcher still talk to us.
        for m in [WM_CUURSOR_RELOAD, WM_CUURSOR_SHOW_SETTINGS] {
            let _ = ChangeWindowMessageFilterEx(hwnd, m, MSGFLT_ALLOW, None);
        }
        hwnd
    }
}

fn tray(action: NOTIFY_ICON_MESSAGE) {
    APP.with(|a| {
        let a = a.borrow();
        let Some(app) = a.as_ref() else { return };
        let on = app.cfg.enabled;
        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: app.hwnd,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: WM_TRAY,
            hIcon: if on { app.icon_on } else { app.icon_off },
            ..Default::default()
        };
        let tip = if on {
            tr("cuursor — включён", "cuursor — on")
        } else {
            tr("cuursor — выключен", "cuursor — off")
        };
        copy_wide(&mut nid.szTip, tip);
        unsafe {
            let _ = Shell_NotifyIconW(action, &nid);
        }
    });
}

fn refresh_icons() {
    let (on, off) = load_icons();
    APP.with(|a| {
        if let Some(app) = a.borrow_mut().as_mut() {
            unsafe {
                let _ = DestroyIcon(app.icon_on);
                let _ = DestroyIcon(app.icon_off);
            }
            app.icon_on = on;
            app.icon_off = off;
        }
    });
    tray(NIM_MODIFY);
}

fn spawn_ui(args: &[&str]) -> Option<Child> {
    let exe = std::env::current_exe().ok()?.with_file_name("cuursor-ui.exe");
    match Command::new(&exe).args(args).spawn() {
        Ok(c) => {
            unsafe {
                let _ = AllowSetForegroundWindow(c.id());
            }
            Some(c)
        }
        Err(e) => {
            message(
                MB_ICONERROR,
                &format!("{}\n{}\n\n{e}", tr("Не удалось открыть окно:", "Could not open:"), exe.display()),
            );
            None
        }
    }
}

fn now_ms() -> u64 {
    filetime_ms(unsafe { GetSystemTimeAsFileTime() })
}

fn filetime_ms(t: FILETIME) -> u64 {
    (((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64) / 10_000
}

/// Open the flyout next to the tray icon, unless the click is the one that
/// just closed it (the flyout closes itself when it loses focus).
fn toggle_flyout(hwnd: HWND) {
    let recently_open = APP.with(|a| {
        let mut a = a.borrow_mut();
        let Some(child) = a.as_mut().and_then(|app| app.flyout.as_mut()) else { return false };
        match child.try_wait() {
            Ok(None) => true,
            _ => unsafe {
                let (mut c, mut e, mut k, mut u) = Default::default();
                let h = HANDLE(child.as_raw_handle());
                GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u).is_ok()
                    && now_ms().saturating_sub(filetime_ms(e)) < REOPEN_GUARD_MS
            },
        }
    });
    if recently_open {
        return;
    }

    let id = NOTIFYICONIDENTIFIER {
        cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
        hWnd: hwnd,
        uID: 1,
        ..Default::default()
    };
    let r = unsafe { Shell_NotifyIconGetRect(&id) }.unwrap_or_else(|_| {
        let mut p = POINT::default();
        let _ = unsafe { GetCursorPos(&mut p) };
        RECT { left: p.x, top: p.y, right: p.x + 1, bottom: p.y + 1 }
    });
    let rect = [r.left, r.top, r.right, r.bottom].map(|v| v.to_string());
    let mut args = vec!["--flyout"];
    args.extend(rect.iter().map(String::as_str));
    let child = spawn_ui(&args);
    APP.with(|a| {
        if let Some(app) = a.borrow_mut().as_mut() {
            app.flyout = child;
        }
    });
}

fn show_menu(hwnd: HWND) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let s = wide(tr("Настройки", "Settings"));
        let x = wide(tr("Выход", "Exit"));
        let _ = AppendMenuW(menu, MF_STRING, ID_SETTINGS, PCWSTR(s.as_ptr()));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, ID_EXIT, PCWSTR(x.as_ptr()));

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON, pt.x, pt.y, None, hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            match lparam.0 as u32 {
                WM_LBUTTONUP => toggle_flyout(hwnd),
                WM_RBUTTONUP => show_menu(hwnd),
                _ => {}
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            match wparam.0 & 0xFFFF {
                ID_SETTINGS => {
                    spawn_ui(&["--settings"]);
                }
                ID_EXIT => unsafe {
                    let _ = DestroyWindow(hwnd);
                },
                _ => {}
            }
            LRESULT(0)
        }
        WM_CUURSOR_RELOAD => {
            reload();
            LRESULT(0)
        }
        WM_CUURSOR_SHOW_SETTINGS => {
            spawn_ui(&["--settings"]);
            LRESULT(0)
        }
        WM_DISPLAYCHANGE => {
            // Several of these arrive while Windows rearranges; settle first.
            unsafe { SetTimer(Some(hwnd), TIMER_RELOAD, 700, None) };
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_RELOAD => {
            let _ = unsafe { KillTimer(Some(hwnd), TIMER_RELOAD) };
            reload();
            refresh_icons();
            LRESULT(0)
        }
        WM_DPICHANGED | WM_SETTINGCHANGE => {
            refresh_icons();
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => {
            let recreated = APP.with(|a| a.try_borrow().ok().and_then(|a| a.as_ref().map(|a| a.taskbar_created)));
            if recreated == Some(msg) && msg != 0 {
                // Explorer restarted; the old icon is gone.
                tray(NIM_ADD);
                return LRESULT(0);
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
    }
}

pub(crate) fn message(kind: MESSAGEBOX_STYLE, text: &str) {
    let t = wide(text);
    unsafe {
        MessageBoxW(None, PCWSTR(t.as_ptr()), w!("cuursor"), MB_OK | kind);
    }
}

pub(crate) fn fatal(text: &str) -> ! {
    message(MB_ICONERROR, text);
    std::process::exit(1);
}

fn copy_wide(dst: &mut [u16], s: &str) {
    let src: Vec<u16> = s.encode_utf16().take(dst.len() - 1).collect();
    dst[..src.len()].copy_from_slice(&src);
    dst[src.len()] = 0;
}
