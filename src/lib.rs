//! cuursor: seamless cursor movement between monitors of different size and
//! resolution. The geometry is platform independent and unit tested; the
//! Windows glue lives in `main.rs` / `win/`.

pub mod config;
pub mod edid;
pub mod layout;
