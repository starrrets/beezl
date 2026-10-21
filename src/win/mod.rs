//! Windows application: tray icon, settings, and wiring to the mouse hook.

mod autostart;
mod hook;
mod icon;
mod monitors;

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

use cuursor::config::{self, Config};
use cuursor::layout::Layout;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcess, SetPriorityClass, ABOVE_NORMAL_PRIORITY_CLASS,
};
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Shell::{
    ShellExecuteW, Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, DispatchMessageW,
    GetCursorPos, GetMessageW, KillTimer, MessageBoxW, PostMessageW, PostQuitMessage, RegisterClassW,
    RegisterWindowMessageW, SetForegroundWindow, SetTimer, TrackPopupMenu, TranslateMessage, HICON, MB_ICONERROR,
    MB_ICONINFORMATION, MB_OK, MF_CHECKED, MF_SEPARATOR, MF_STRING, MSG, SW_SHOWNORMAL, TPM_RIGHTBUTTON,
    WINDOW_EX_STYLE, WM_APP, WM_COMMAND, WM_CONTEXTMENU, WM_DESTROY, WM_DISPLAYCHANGE, WM_LBUTTONDBLCLK, WM_NULL,
    WM_RBUTTONUP, WM_TIMER, WNDCLASSW, WS_OVERLAPPED,
};

const WM_TRAY: u32 = WM_APP + 10;
const TIMER_RELOAD: usize = 1;

const ID_ENABLED: usize = 1;
const ID_MONITORS: usize = 2;
const ID_CONFIG: usize = 3;
const ID_RELOAD: usize = 4;
const ID_AUTOSTART: usize = 5;
const ID_EXIT: usize = 6;

struct App {
    hwnd: HWND,
    cfg: Config,
    monitors: Vec<monitors::Monitor>,
    icon_on: HICON,
    icon_off: HICON,
    taskbar_created: u32,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

pub fn run() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = CreateMutexW(None, true, w!("Local\\cuursor-single-instance"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            message(MB_ICONINFORMATION, "cuursor уже запущен — ищите значок в трее.");
            return;
        }
        let _ = SetPriorityClass(GetCurrentProcess(), ABOVE_NORMAL_PRIORITY_CLASS);
    }

    let cfg = load_config();
    hook::ENABLED.store(cfg.enabled, Ordering::Relaxed);
    let mons = monitors::enumerate(&cfg);
    hook::start(params(&cfg, &mons));
    autostart::refresh_path();

    let hwnd = create_window();
    APP.with(|a| {
        *a.borrow_mut() = Some(App {
            hwnd,
            cfg,
            monitors: mons,
            icon_on: icon::make(true),
            icon_off: icon::make(false),
            taskbar_created: unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) },
        })
    });
    tray(NIM_ADD);

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

fn params(cfg: &Config, mons: &[monitors::Monitor]) -> hook::Params {
    hook::Params {
        layout: Layout::build(mons.iter().map(|m| m.screen()).collect(), cfg.mode, cfg.align),
        skip_injected: cfg.skip_injected,
        max_jump_px: cfg.max_jump_px.max(1),
    }
}

fn config_path() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("cuursor").join("config.toml")
}

fn load_config() -> Config {
    let path = config_path();
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&path, config::DEFAULT_TOML);
            config::DEFAULT_TOML.to_string()
        }
    };
    config::parse(&text).unwrap_or_else(|e| {
        message(MB_ICONERROR, &format!("Ошибка в {}:\n\n{e}\n\nИспользую настройки по умолчанию.", path.display()));
        Config::default()
    })
}

/// Re-read config and monitors and hand a fresh layout to the hook.
fn reload(reread_config: bool) {
    let cfg = if reread_config { Some(load_config()) } else { None };
    let p = APP.with(|a| {
        let mut a = a.borrow_mut();
        let app = a.as_mut()?;
        if let Some(cfg) = cfg {
            hook::ENABLED.store(cfg.enabled, Ordering::Relaxed);
            app.cfg = cfg;
        }
        app.monitors = monitors::enumerate(&app.cfg);
        Some(params(&app.cfg, &app.monitors))
    });
    if let Some(p) = p {
        hook::update(p);
    }
    tray(NIM_MODIFY);
}

fn create_window() -> HWND {
    unsafe {
        let instance = GetModuleHandleW(None).expect("module handle");
        let class = w!("cuursor.tray");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassW(&wc);
        // A real (hidden) top-level window: message-only windows miss WM_DISPLAYCHANGE.
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
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
        .expect("create window")
    }
}

fn tray(action: windows::Win32::UI::Shell::NOTIFY_ICON_MESSAGE) {
    APP.with(|a| {
        let a = a.borrow();
        let Some(app) = a.as_ref() else { return };
        let on = hook::ENABLED.load(Ordering::Relaxed);
        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: app.hwnd,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: WM_TRAY,
            hIcon: if on { app.icon_on } else { app.icon_off },
            ..Default::default()
        };
        let tip = if on { "cuursor — включён" } else { "cuursor — выключен" };
        copy_wide(&mut nid.szTip, tip);
        unsafe {
            let _ = Shell_NotifyIconW(action, &nid);
        }
    });
}

fn show_menu(hwnd: HWND) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let checked = |b: bool| if b { MF_STRING | MF_CHECKED } else { MF_STRING };
        let _ = AppendMenuW(menu, checked(hook::ENABLED.load(Ordering::Relaxed)), ID_ENABLED, w!("Включено"));
        let _ = AppendMenuW(menu, MF_STRING, ID_MONITORS, w!("Мониторы…"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, ID_CONFIG, w!("Открыть настройки"));
        let _ = AppendMenuW(menu, MF_STRING, ID_RELOAD, w!("Перечитать настройки"));
        let _ = AppendMenuW(menu, checked(autostart::is_enabled()), ID_AUTOSTART, w!("Запускать при входе в Windows"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, ID_EXIT, w!("Выход"));

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON, pt.x, pt.y, None, hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
    }
}

fn toggle() {
    hook::ENABLED.fetch_xor(true, Ordering::Relaxed);
    tray(NIM_MODIFY);
}

fn command(hwnd: HWND, id: usize) {
    match id {
        ID_ENABLED => toggle(),
        ID_MONITORS => {
            let text = APP.with(|a| a.borrow().as_ref().map(describe)).unwrap_or_default();
            message(MB_ICONINFORMATION, &text);
        }
        ID_CONFIG => {
            let path = wide(&config_path().to_string_lossy());
            unsafe {
                ShellExecuteW(None, w!("open"), w!("notepad.exe"), PCWSTR(path.as_ptr()), None, SW_SHOWNORMAL);
            }
        }
        ID_RELOAD => reload(true),
        ID_AUTOSTART => autostart::set(!autostart::is_enabled()),
        ID_EXIT => unsafe {
            let _ = DestroyWindow(hwnd);
        },
        _ => {}
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            match lparam.0 as u32 {
                WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd),
                WM_LBUTTONDBLCLK => toggle(),
                _ => {}
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            command(hwnd, wparam.0 & 0xFFFF);
            LRESULT(0)
        }
        WM_DISPLAYCHANGE => {
            // Several of these arrive while Windows rearranges; settle first.
            unsafe { SetTimer(Some(hwnd), TIMER_RELOAD, 700, None) };
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_RELOAD => {
            let _ = unsafe { KillTimer(Some(hwnd), TIMER_RELOAD) };
            reload(false);
            LRESULT(0)
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

fn describe(app: &App) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    for (i, m) in app.monitors.iter().enumerate() {
        let (w, h) = (m.rect.width(), m.rect.height());
        let diag = m.size_mm.0.hypot(m.size_mm.1) / 25.4;
        let ppi = w as f64 / (m.size_mm.0 / 25.4);
        let src = match m.source {
            monitors::SizeSource::Edid => "из EDID",
            monitors::SizeSource::Config => "из настроек",
            monitors::SizeSource::Guess => "НЕ ОПРЕДЕЛЁН, задайте в настройках",
        };
        let _ = writeln!(s, "{}. {}{}", i + 1, m.device, if m.primary { " (основной)" } else { "" });
        if !m.model.is_empty() || !m.id.is_empty() {
            let _ = writeln!(s, "    {} [{}]", m.model, m.id);
        }
        let _ = writeln!(s, "    {w}×{h} px, начало ({}, {})", m.rect.left, m.rect.top);
        let _ = writeln!(s, "    {:.0}×{:.0} мм, {diag:.1}\", {ppi:.0} PPI — {src}\n", m.size_mm.0, m.size_mm.1);
    }
    let _ = write!(
        s,
        "Режим: {:?}, выравнивание: {:?}\nНастройки: {}",
        app.cfg.mode,
        app.cfg.align,
        config_path().display()
    );
    s
}

pub(crate) fn message(kind: windows::Win32::UI::WindowsAndMessaging::MESSAGEBOX_STYLE, text: &str) {
    let t = wide(text);
    unsafe {
        MessageBoxW(None, PCWSTR(t.as_ptr()), w!("cuursor"), MB_OK | kind);
    }
}

pub(crate) fn fatal(text: &str) -> ! {
    message(MB_ICONERROR, text);
    std::process::exit(1);
}

pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub(crate) fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

fn copy_wide(dst: &mut [u16], s: &str) {
    let src: Vec<u16> = s.encode_utf16().take(dst.len() - 1).collect();
    dst[..src.len()].copy_from_slice(&src);
    dst[src.len()] = 0;
}
