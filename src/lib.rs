//! cuursor: seamless cursor movement between monitors of different size and
//! resolution. The geometry is platform independent and unit tested; the
//! Windows glue lives in `sys` (shared), `main.rs` / `win/` (background
//! process) and `bin/cuursor-ui` (settings and tray flyout).

pub mod config;
pub mod edid;
pub mod layout;
#[cfg(windows)]
pub mod sys;
pub mod task;
