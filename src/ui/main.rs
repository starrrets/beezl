//! cuursor-ui: the tray flyout (`--flyout L T R B`, the tray icon rectangle),
//! the settings window (`--settings`), and setup: run from anywhere but the
//! install folder it is the installer, `--uninstall` removes the app.

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("cuursor-ui runs on Windows only.");
    std::process::exit(1);
}

#[cfg(windows)]
fn main() {
    app::run();
}

#[cfg(windows)]
mod install;

#[cfg(windows)]
mod app {
    slint::include_modules!();

    use crate::install;

    use std::cell::RefCell;
    use std::rc::Rc;

    use cuursor::config::{Align, Config, Mode};
    use cuursor::sys::{self, autostart, monitors, monitors::Monitor, monitors::SizeSource, tr, wide};
    use slint::winit_030::{winit, EventResult, WinitWindowAccessor};
    use slint::{ComponentHandle, Model, ModelRc, PhysicalPosition, SharedString, VecModel};
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HWND, RECT};
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromRect, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows::Win32::UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, FindWindowW, GetForegroundWindow, SetForegroundWindow, ShowWindow, ASFW_ANY,
        SW_RESTORE,
    };

    const FLYOUT_W: f32 = 360.0;
    const FLYOUT_H: f32 = 350.0;
    const GAP: f32 = 12.0;
    const WINDOW_TITLE_RU: &str = "Настройки cuursor";
    const WINDOW_TITLE_EN: &str = "cuursor settings";

    pub fn run() {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let has = |a: &str| args.iter().any(|x| x == a);

        // Elevated workers of the setup and uninstall windows: no UI.
        if has("--do-install") || has("--do-uninstall") {
            std::process::exit(install::worker(has("--do-install")));
        }
        let mode = if has("--uninstall") {
            RunMode::Uninstall
        } else if args.is_empty() && install::has_payload() && !install::running_installed() {
            RunMode::Setup
        } else {
            RunMode::App
        };
        // The uninstaller removes its own folder, so it runs from a temp copy.
        if mode == RunMode::Uninstall && install::running_installed() {
            if let Ok(exe) = std::env::current_exe() {
                let tmp = std::env::temp_dir().join("cuursor-uninstall.exe");
                if std::fs::copy(&exe, &tmp).is_ok()
                    && std::process::Command::new(&tmp).arg("--uninstall").spawn().is_ok()
                {
                    return;
                }
            }
        }
        let flyout_rect = match args.first().map(String::as_str) {
            Some("--flyout") => {
                let n: Vec<i32> = args[1..].iter().filter_map(|s| s.parse().ok()).collect();
                Some(if n.len() == 4 {
                    RECT { left: n[0], top: n[1], right: n[2], bottom: n[3] }
                } else {
                    RECT::default()
                })
            }
            _ => None,
        };

        // The flyout must not show up in the taskbar or Alt+Tab.
        let is_flyout = flyout_rect.is_some();
        let backend = slint::BackendSelector::new().backend_name("winit".into()).with_winit_window_attributes_hook(
            move |attrs| {
                use winit::platform::windows::WindowAttributesExtWindows;
                if is_flyout {
                    attrs.with_skip_taskbar(true)
                } else {
                    attrs
                }
            },
        );
        if backend.select().is_err() {
            return;
        }

        match mode {
            RunMode::Setup => return setup(),
            RunMode::Uninstall => return uninstall(),
            RunMode::App => {}
        }
        let cfg = Rc::new(RefCell::new(sys::load_config()));
        match flyout_rect {
            Some(r) => flyout(cfg, r),
            None => settings(cfg),
        }
    }

    #[derive(PartialEq)]
    enum RunMode {
        App,
        Setup,
        Uninstall,
    }

    fn setup_strings(g: SetupStrings<'_>) {
        if !sys::is_russian() {
            return;
        }
        g.set_tagline("Плавный переход курсора между мониторами разного размера".into());
        g.set_install("Установить".into());
        g.set_update("Обновить".into());
        g.set_installing("Установка…".into());
        g.set_done("cuursor установлен".into());
        g.set_done_desc("Он уже работает в трее и будет запускаться вместе с Windows.".into());
        g.set_open("Открыть настройки".into());
        g.set_close("Закрыть".into());
        g.set_failed("Не получилось".into());
        g.set_retry("Попробовать снова".into());
        g.set_uninstall_q("Удалить cuursor?".into());
        g.set_uninstall_desc("Программа и её настройки будут удалены с компьютера.".into());
        g.set_uninstall("Удалить".into());
        g.set_cancel("Отмена".into());
        g.set_uninstalling("Удаление…".into());
        g.set_uninstalled("cuursor удалён".into());
    }

    fn worker_args(verb: &str) -> String {
        format!("{verb} --parent {}", std::process::id())
    }

    fn setup() {
        let ui = Setup::new().expect("setup");
        setup_strings(ui.global::<SetupStrings>());
        let version = env!("CARGO_PKG_VERSION");
        ui.set_version(version.into());
        let dir = install::install_dir();
        let details = match install::installed_version() {
            Some(old) if old == version => {
                ui.set_is_update(true);
                format!("{} {version} · {}", tr("Уже установлена версия", "Already installed:"), dir.display())
            }
            Some(old) => {
                ui.set_is_update(true);
                format!("{} {old} → {version} · {}", tr("Обновление", "Update"), dir.display())
            }
            None => format!("{} {}", tr("Папка установки:", "Install folder:"), dir.display()),
        };
        ui.set_details(details.into());

        {
            let weak = ui.as_weak();
            ui.on_install(move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_page(1);
                }
                let weak = weak.clone();
                std::thread::spawn(move || {
                    let res = install::run_elevated(&worker_args("--do-install"));
                    if res.is_ok() {
                        // Start it the normal way; it relaunches itself through the
                        // logon task to run elevated.
                        let _ = std::process::Command::new(install::install_dir().join("cuursor.exe")).spawn();
                    }
                    let _ = weak.upgrade_in_event_loop(move |ui| match res {
                        Ok(()) => ui.set_page(2),
                        Err(e) => {
                            ui.set_error_text(e.into());
                            ui.set_page(3);
                        }
                    });
                });
            });
        }
        ui.on_open_settings(|| {
            unsafe {
                let _ = AllowSetForegroundWindow(ASFW_ANY);
            }
            let _ = std::process::Command::new(install::install_dir().join("cuursor-ui.exe")).arg("--settings").spawn();
            let _ = slint::quit_event_loop();
        });
        ui.on_dismiss(|| {
            let _ = slint::quit_event_loop();
        });
        let _ = ui.run();
    }

    fn uninstall() {
        let ui = Uninstall::new().expect("uninstall");
        setup_strings(ui.global::<SetupStrings>());
        {
            let weak = ui.as_weak();
            ui.on_uninstall(move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_page(1);
                }
                let weak = weak.clone();
                std::thread::spawn(move || {
                    let res = install::run_elevated(&worker_args("--do-uninstall"));
                    let _ = weak.upgrade_in_event_loop(move |ui| match res {
                        Ok(()) => ui.set_page(2),
                        Err(e) => {
                            ui.set_error_text(e.into());
                            ui.set_page(3);
                        }
                    });
                });
            });
        }
        ui.on_dismiss(|| {
            let _ = slint::quit_event_loop();
        });
        let _ = ui.run();
    }

    macro_rules! apply_strings {
        ($ui:expr) => {{
            // English defaults live in app.slint.
            if sys::is_russian() {
                let g = $ui.global::<Strings>();
                g.set_on("Включён".into());
                g.set_off("Выключен".into());
                g.set_smooth("Плавный переход курсора".into());
                g.set_smooth_desc("Без прыжков и залипаний на краях".into());
                g.set_settings("Настройки".into());
                g.set_size_warning("Размер одного из мониторов не определён. Укажите его в настройках.".into());
                g.set_settings_title("Настройки".into());
                g.set_window_title(WINDOW_TITLE_RU.into());
                g.set_general("Основное".into());
                g.set_enabled("Включено".into());
                g.set_enabled_desc("Исправлять переход курсора между мониторами".into());
                g.set_autostart("Запускать вместе с Windows".into());
                g.set_crossing("Переход курсора".into());
                g.set_mode("Способ".into());
                g.set_modes(strings_model(&["По физическому размеру", "Растянуть край на край"]));
                g.set_align("Выравнивание мониторов".into());
                g.set_align_desc("Как экраны стоят на столе".into());
                g.set_aligns(strings_model(&[
                    "Как в параметрах дисплея",
                    "По верхнему краю",
                    "По центру",
                    "По нижнему краю",
                ]));
                g.set_monitors("Мониторы".into());
                g.set_diagonal_desc("Диагональ, дюймы".into());
            }
        }};
    }

    fn strings_model(items: &[&str]) -> ModelRc<SharedString> {
        ModelRc::new(VecModel::from(items.iter().map(|s| SharedString::from(*s)).collect::<Vec<_>>()))
    }

    /// Save, tell the background process, and return the fresh monitor list.
    fn commit(cfg: &Config) -> Vec<Monitor> {
        let _ = sys::save_config(cfg);
        sys::notify_daemon(sys::WM_CUURSOR_RELOAD);
        monitors::enumerate(cfg)
    }

    /// Monitors scaled into a `box_w` × `box_h` box, keeping the Windows arrangement.
    fn tiles(mons: &[Monitor], box_w: f32, box_h: f32) -> ModelRc<MonitorTile> {
        let mut v = Vec::new();
        if let (Some(l), Some(t), Some(r), Some(b)) = (
            mons.iter().map(|m| m.rect.left).min(),
            mons.iter().map(|m| m.rect.top).min(),
            mons.iter().map(|m| m.rect.right).max(),
            mons.iter().map(|m| m.rect.bottom).max(),
        ) {
            let pad = 14.0;
            let (bw, bh) = ((r - l) as f32, (b - t) as f32);
            let k = ((box_w - 2.0 * pad) / bw).min((box_h - 2.0 * pad) / bh);
            let (ox, oy) = ((box_w - bw * k) / 2.0, (box_h - bh * k) / 2.0);
            let gap = 2.0;
            for m in mons {
                v.push(MonitorTile {
                    x: ox + (m.rect.left - l) as f32 * k + gap,
                    y: oy + (m.rect.top - t) as f32 * k + gap,
                    w: m.rect.width() as f32 * k - 2.0 * gap,
                    h: m.rect.height() as f32 * k - 2.0 * gap,
                    title: format!("{:.1}″", m.diagonal_in()).replace(".0″", "″").into(),
                    subtitle: format!("{}×{}", m.rect.width(), m.rect.height()).into(),
                    primary: m.primary,
                    warn: m.source == SizeSource::Guess,
                });
            }
        }
        ModelRc::new(VecModel::from(v))
    }

    fn rows(mons: &[Monitor]) -> Vec<MonitorRow> {
        mons.iter()
            .enumerate()
            .map(|(i, m)| {
                let mut detail = format!("{}×{}", m.rect.width(), m.rect.height());
                if m.primary {
                    detail.push_str(tr(" · основной", " · primary"));
                }
                let (hint, warn) = match m.source {
                    SizeSource::Edid => (tr("определено автоматически", "detected"), false),
                    SizeSource::Config => (tr("задано вручную", "set manually"), false),
                    SizeSource::Guess => (tr("не определено, укажите", "unknown, please set"), true),
                };
                MonitorRow {
                    name: m.display_name(i).into(),
                    detail: detail.into(),
                    diagonal: if m.source == SizeSource::Guess {
                        SharedString::new()
                    } else {
                        format!("{:.1}", m.diagonal_in()).into()
                    },
                    hint: hint.into(),
                    warn,
                }
            })
            .collect()
    }

    fn hwnd_of(window: &slint::Window) -> Option<HWND> {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        window
            .with_winit_window(|w| match w.window_handle().ok()?.as_raw() {
                RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as *mut _)),
                _ => None,
            })
            .flatten()
    }

    fn flyout(cfg: Rc<RefCell<Config>>, icon: RECT) {
        let ui = Flyout::new().expect("flyout");
        apply_strings!(ui);
        let mons = monitors::enumerate(&cfg.borrow());
        ui.set_tiles(tiles(&mons, FLYOUT_W - 32.0, 116.0));
        ui.set_size_warning(mons.iter().any(|m| m.source == SizeSource::Guess));
        ui.set_enabled(cfg.borrow().enabled);

        {
            let cfg = cfg.clone();
            ui.on_toggled(move |on| {
                let mut c = cfg.borrow_mut();
                c.enabled = on;
                commit(&c);
            });
        }
        ui.on_open_settings(|| {
            // We are the foreground process, so the new window may come to front.
            unsafe {
                let _ = AllowSetForegroundWindow(ASFW_ANY);
            }
            if let Ok(exe) = std::env::current_exe() {
                let _ = std::process::Command::new(exe).arg("--settings").spawn();
            }
            let _ = slint::quit_event_loop();
        });
        ui.on_dismiss(|| {
            let _ = slint::quit_event_loop();
        });

        // Close when focus moves elsewhere, like the system flyouts.
        ui.window().on_winit_window_event(|_, e| {
            if let winit::event::WindowEvent::Focused(false) = e {
                let _ = slint::quit_event_loop();
            }
            EventResult::Propagate
        });

        // Fallback for when Windows refused to give us focus: close once
        // something else is in front.
        let watchdog = slint::Timer::default();
        {
            let ui = ui.as_weak();
            let started = std::time::Instant::now();
            watchdog.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(250), move || {
                let Some(ui) = ui.upgrade() else { return };
                let ours = hwnd_of(ui.window());
                let front = unsafe { GetForegroundWindow() };
                if started.elapsed().as_millis() > 800 && ours.is_some_and(|h| h != front) {
                    let _ = slint::quit_event_loop();
                }
            });
        }

        let (x, y) = flyout_position(icon);
        ui.window().set_position(PhysicalPosition::new(x, y));
        let _ = ui.show();
        if let Some(h) = hwnd_of(ui.window()) {
            unsafe {
                let pref = DWMWCP_ROUND;
                let _ = DwmSetWindowAttribute(
                    h,
                    DWMWA_WINDOW_CORNER_PREFERENCE,
                    &pref as *const _ as *const _,
                    std::mem::size_of_val(&pref) as u32,
                );
                let _ = SetForegroundWindow(h);
            }
        }
        let _ = slint::run_event_loop();
    }

    /// Next to the tray icon, inside the work area of its monitor.
    fn flyout_position(icon: RECT) -> (i32, i32) {
        unsafe {
            let mon = MonitorFromRect(&icon, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(mon, &mut mi);
            let (mut dx, mut dy) = (96u32, 96u32);
            let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
            let s = dx as f32 / 96.0;
            let (w, h, gap) = ((FLYOUT_W * s) as i32, (FLYOUT_H * s) as i32, (GAP * s) as i32);
            let wa = mi.rcWork;
            let cx = (icon.left + icon.right) / 2;
            let cy = (icon.top + icon.bottom) / 2;
            let x = (cx - w / 2).clamp(wa.left + gap, (wa.right - w - gap).max(wa.left + gap));
            // Taskbar at the bottom (or the icon is in the lower half): open upwards.
            let y = if cy > (wa.top + wa.bottom) / 2 {
                wa.bottom.min(icon.top) - h - gap
            } else {
                wa.top.max(icon.bottom) + gap
            };
            (x, y.clamp(wa.top + gap, (wa.bottom - h - gap).max(wa.top + gap)))
        }
    }

    fn settings(cfg: Rc<RefCell<Config>>) {
        // One settings window: a second request brings the first to front.
        unsafe {
            let _ = CreateMutexW(None, true, w!("Local\\cuursor-settings"));
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let title = wide(tr(WINDOW_TITLE_RU, WINDOW_TITLE_EN));
                if let Ok(h) = FindWindowW(PCWSTR::null(), PCWSTR(title.as_ptr())) {
                    let _ = ShowWindow(h, SW_RESTORE);
                    let _ = SetForegroundWindow(h);
                }
                return;
            }
        }

        let ui = Settings::new().expect("settings");
        apply_strings!(ui);
        let model = Rc::new(VecModel::<MonitorRow>::default());
        ui.set_monitors(ModelRc::from(model.clone()));
        let mons = Rc::new(RefCell::new(Vec::<Monitor>::new()));

        let refresh = {
            let ui = ui.as_weak();
            let model = model.clone();
            let mons = mons.clone();
            move |fresh: Vec<Monitor>, rebuild_rows: bool| {
                let Some(ui) = ui.upgrade() else { return };
                ui.set_tiles(tiles(&fresh, 472.0, 160.0));
                let new_rows = rows(&fresh);
                if rebuild_rows || model.row_count() != new_rows.len() {
                    model.set_vec(new_rows);
                } else {
                    // Keep what is being typed; only refresh the hints.
                    for (i, r) in new_rows.into_iter().enumerate() {
                        if let Some(mut old) = model.row_data(i) {
                            old.hint = r.hint;
                            old.warn = r.warn;
                            model.set_row_data(i, old);
                        }
                    }
                }
                *mons.borrow_mut() = fresh;
            }
        };
        let refresh = Rc::new(refresh);

        {
            let c = cfg.borrow();
            ui.set_enabled(c.enabled);
            ui.set_autostart(autostart::is_enabled(&c));
            ui.set_mode(if c.mode == Mode::Proportional { 1 } else { 0 });
            ui.set_align(match c.align {
                Align::Auto => 0,
                Align::Start => 1,
                Align::Center => 2,
                Align::End => 3,
            });
            refresh(monitors::enumerate(&c), true);
        }

        {
            let cfg = cfg.clone();
            ui.on_enabled_toggled(move |on| {
                let mut c = cfg.borrow_mut();
                c.enabled = on;
                commit(&c);
            });
        }
        {
            let cfg = cfg.clone();
            ui.on_autostart_toggled(move |on| {
                let mut c = cfg.borrow_mut();
                autostart::set_enabled(&mut c, on);
                commit(&c);
            });
        }
        {
            let cfg = cfg.clone();
            ui.on_mode_selected(move |i| {
                let mut c = cfg.borrow_mut();
                c.mode = if i == 1 { Mode::Proportional } else { Mode::Physical };
                commit(&c);
            });
        }
        {
            let cfg = cfg.clone();
            ui.on_align_selected(move |i| {
                let mut c = cfg.borrow_mut();
                c.align = match i {
                    1 => Align::Start,
                    2 => Align::Center,
                    3 => Align::End,
                    _ => Align::Auto,
                };
                commit(&c);
            });
        }
        {
            let cfg = cfg.clone();
            let mons = mons.clone();
            let refresh = refresh.clone();
            ui.on_diagonal_edited(move |i, text| {
                let Some(key) = mons.borrow().get(i as usize).map(|m| m.key.clone()) else { return };
                let text = text.trim().replace(',', ".");
                let value = if text.is_empty() {
                    None
                } else {
                    match text.parse::<f64>() {
                        Ok(d) if (5.0..=150.0).contains(&d) => Some(d),
                        _ => return, // still typing
                    }
                };
                let fresh = {
                    let mut c = cfg.borrow_mut();
                    c.set_diagonal(&key, value);
                    commit(&c)
                };
                refresh(fresh, false);
            });
        }

        // Pick up monitors plugged in or rearranged while the window is open.
        let timer = slint::Timer::default();
        {
            let cfg = cfg.clone();
            let mons = mons.clone();
            let refresh = refresh.clone();
            timer.start(slint::TimerMode::Repeated, std::time::Duration::from_secs(2), move || {
                let fresh = monitors::enumerate(&cfg.borrow());
                let changed = {
                    let old = mons.borrow();
                    old.len() != fresh.len() || old.iter().zip(&fresh).any(|(a, b)| a.key != b.key || a.rect != b.rect)
                };
                if changed {
                    refresh(fresh, true);
                }
            });
        }

        let _ = ui.show();
        if let Some(h) = hwnd_of(ui.window()) {
            unsafe {
                let _ = SetForegroundWindow(h);
            }
        }
        let _ = ui.run();
    }
}
