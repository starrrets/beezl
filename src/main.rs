#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod win;

#[cfg(windows)]
fn main() {
    win::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("beezl runs on Windows only; on this platform only `cargo test` is useful.");
    std::process::exit(1);
}
