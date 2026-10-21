//! The low-level mouse hook. It runs on its own high-priority thread so the
//! tray UI can never delay mouse input; the hook does no allocation and no
//! I/O, and only makes syscalls when the cursor is actually leaving a screen.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::Mutex;
use std::thread::JoinHandle;

use beezl::layout::{Decision, Layout, Rect};
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    GetCurrentThread, GetCurrentThreadId, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetClipCursor, GetCursorPos, GetMessageW, PeekMessageW, PostThreadMessageW, SetCursorPos, SetTimer,
    SetWindowsHookExW, UnhookWindowsHookEx, HC_ACTION, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, PM_NOREMOVE, WH_MOUSE_LL,
    WM_APP, WM_MOUSEMOVE, WM_QUIT, WM_TIMER, WM_USER,
};

/// Everything the hook needs, rebuilt by the UI thread and handed over whole.
pub struct Params {
    pub layout: Layout,
    pub skip_injected: bool,
    pub max_jump_px: i32,
}

pub static ENABLED: AtomicBool = AtomicBool::new(true);
static THREAD_ID: AtomicU32 = AtomicU32::new(0);
static THREAD: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

const MSG_UPDATE: u32 = WM_APP + 1;
const REHOOK_MS: u32 = 60_000;
/// Larger than any single mouse event's travel when crossing an edge.
const EDGE_MARGIN: i32 = 128;

struct State {
    p: Params,
    /// Bounding box of all screens; a smaller clip rect means a game or app
    /// has confined the cursor and we must not move it out.
    bounds: Rect,
    /// Screen the cursor was last seen on (cache for the fast path).
    cur: usize,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

impl State {
    fn new(p: Params) -> Self {
        let bounds = p.layout.screens.iter().fold(Rect::new(i32::MAX, i32::MAX, i32::MIN, i32::MIN), |b, s| {
            Rect::new(
                b.left.min(s.rect.left),
                b.top.min(s.rect.top),
                b.right.max(s.rect.right),
                b.bottom.max(s.rect.bottom),
            )
        });
        Self { p, bounds, cur: 0 }
    }

    #[inline(always)]
    fn handle(&mut self, info: &MSLLHOOKSTRUCT) -> Option<(i32, i32)> {
        let (x, y) = (info.pt.x, info.pt.y);
        let layout = &self.p.layout;
        // Fast path: well inside the cached screen. This is almost every event.
        // Near an edge we always re-check the real position: the cache goes
        // stale when someone else teleports the cursor with SetCursorPos
        // (the hook never sees that), and a crossing would then look like a
        // move within the old screen.
        match layout.screens.get(self.cur) {
            Some(s) if s.rect.contains_inset(x, y, EDGE_MARGIN) => return None,
            None => return None,
            _ => {}
        }
        if self.p.skip_injected && info.flags & LLMHF_INJECTED != 0 {
            return None;
        }

        // The hook sees the move before it happens, so this is where the
        // cursor is now.
        let mut pos = POINT::default();
        unsafe { GetCursorPos(&mut pos) }.ok()?;
        let src = layout.screen_at(pos.x, pos.y)?;
        self.cur = src;

        if (x - pos.x).abs() > self.p.max_jump_px || (y - pos.y).abs() > self.p.max_jump_px || self.clipped() {
            self.cur = layout.screen_at(x, y).unwrap_or(src);
            return None;
        }

        match layout.on_exit(src, x, y) {
            Decision::Pass => {
                self.cur = layout.screen_at(x, y).unwrap_or(src);
                None
            }
            Decision::Move(nx, ny) => {
                self.cur = layout.screen_at(nx, ny).unwrap_or(src);
                Some((nx, ny))
            }
        }
    }

    fn clipped(&self) -> bool {
        let mut r = RECT::default();
        if unsafe { GetClipCursor(&mut r) }.is_err() {
            return false;
        }
        r.left > self.bounds.left
            || r.top > self.bounds.top
            || r.right < self.bounds.right
            || r.bottom < self.bounds.bottom
    }
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && wparam.0 == WM_MOUSEMOVE as usize && ENABLED.load(Ordering::Relaxed) {
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        let target = STATE.with(|s| s.borrow_mut().as_mut().and_then(|st| st.handle(info)));
        if let Some((x, y)) = target {
            // Swallow the original move and place the cursor ourselves.
            let _ = unsafe { SetCursorPos(x, y) };
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

pub fn start(p: Params) {
    let (tx, rx) = mpsc::channel();
    let handle = std::thread::Builder::new()
        .name("mouse-hook".into())
        .spawn(move || unsafe {
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);
            STATE.with(|s| *s.borrow_mut() = Some(State::new(p)));

            let module: Option<HINSTANCE> = GetModuleHandleW(None).ok().map(Into::into);
            let mut hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module, 0);

            // Make sure the thread has a message queue before anyone posts to it.
            let mut msg = MSG::default();
            let _ = PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE);
            let _ = tx.send((GetCurrentThreadId(), hook.is_ok()));
            // Windows silently removes a low-level hook that once took too long
            // (e.g. the machine stalled under load), so renew it periodically.
            let _ = SetTimer(None, 0, REHOOK_MS, None);

            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                match msg.message {
                    MSG_UPDATE => {
                        let p = Box::from_raw(msg.lParam.0 as *mut Params);
                        STATE.with(|s| *s.borrow_mut() = Some(State::new(*p)));
                    }
                    WM_TIMER => {
                        if let Ok(new) = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module, 0) {
                            if let Ok(old) = std::mem::replace(&mut hook, Ok(new)) {
                                let _ = UnhookWindowsHookEx(old);
                            }
                        }
                    }
                    _ => {}
                }
            }
            if let Ok(h) = hook {
                let _ = UnhookWindowsHookEx(h);
            }
        })
        .expect("spawn hook thread");

    let (tid, ok) = rx.recv().expect("hook thread start");
    if !ok {
        super::fatal(beezl::sys::tr("Не удалось установить хук мыши.", "Could not install the mouse hook."));
    }
    THREAD_ID.store(tid, Ordering::Release);
    *THREAD.lock().unwrap() = Some(handle);
}

pub fn update(p: Params) {
    let ptr = Box::into_raw(Box::new(p));
    let tid = THREAD_ID.load(Ordering::Acquire);
    if unsafe { PostThreadMessageW(tid, MSG_UPDATE, WPARAM(0), LPARAM(ptr as isize)) }.is_err() {
        drop(unsafe { Box::from_raw(ptr) });
    }
}

pub fn stop() {
    let tid = THREAD_ID.load(Ordering::Acquire);
    let _ = unsafe { PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0)) };
    if let Some(h) = THREAD.lock().unwrap().take() {
        let _ = h.join();
    }
}
