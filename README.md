# beezl

**English** · [Русский](README.ru.md)

A Windows app that makes the mouse cursor cross between monitors of different
size and resolution smoothly: no jumps and no getting stuck at the edges.
Written in Rust. The background process is under 1 MB and does no work until
the cursor reaches the edge of a screen.

Up to v0.3 the app was called **cuursor**; installing beezl over it moves
your settings and removes the old version.

## Download

[**Download beezl-setup.exe**](../../releases/latest/download/beezl-setup.exe)
(latest version; all versions are on the
[Releases](../../releases) page).

1. Run `beezl-setup.exe` and click **Install**. Windows asks for
   administrator rights once: beezl needs them to keep working over windows
   that run as administrator (Task Manager, installers, consoles).
   If Windows shows "Windows protected your PC", click
   **More info → Run anyway** (see [Code signing](#code-signing)).
2. Done: the icon appears in the system tray, and beezl starts with Windows.

Uninstall it like any other app: **Settings → Apps**.

## Usage

* **Click the tray icon** to open a panel with an on/off switch and a map of
  your monitors.
* **Right-click** for **Settings** and **Exit**.
* **Settings** has only what matters: on/off, start with Windows, crossing
  method, monitor alignment, and the diagonal of each monitor (in case Windows
  could not detect it).

Arrange your monitors in **Settings → System → Display** the way they stand on
your desk (for example, aligned by their bottom edges). beezl takes the
alignment from there.

## How it works

Windows joins monitors by pixels, not by centimeters. When neighbouring screens
have different pixel density (say, a 24" Full HD next to a 27" 1440p), the
cursor jumps several centimeters at the border and gets stuck on part of the
edge.

beezl installs a low-level mouse hook (`WH_MOUSE_LL`) on its own
high-priority thread. While the cursor is inside a monitor, the hook does one
comparison and lets the event through. At a border the position is converted to
millimeters (the panel size comes from the monitor's EDID), and the cursor is
placed at the same physical point on the neighbouring screen. Parts of an edge
with nothing physically next to them are clamped to the nearest point of the
neighbour, so the cursor never gets stuck.

It works with any number of monitors in any arrangement (left, right, above,
below), with mixed scaling and rotated screens, and picks up monitors being
plugged in or out on the fly. It stays out of the way when a game or app
confines the cursor to its window, and ignores synthetic input (Remote Desktop,
graphics tablets).

## Code signing

Builds are not code-signed yet, so SmartScreen warns on first run. Every
release is built from source by
[GitHub Actions](../../actions), and
`SHA256SUMS.txt` next to `beezl-setup.exe` lists its checksum. beezl never
sends anything over the network.

## Building from source

```powershell
winget install Rustlang.Rustup
cargo build --release      # target\release\beezl.exe and beezl-ui.exe
cargo test                 # geometry tests
```

The installer is `beezl-ui.exe` with `beezl.exe` embedded:

```powershell
mkdir target\payload; copy target\release\beezl.exe target\payload\
$env:BEEZL_PAYLOAD = "$PWD\target\payload\beezl.exe"
cargo build --release --bin beezl-ui   # target\release\beezl-ui.exe = beezl-setup.exe
```

A new version is released by bumping `version` in `Cargo.toml`: when that
reaches `main`, CI tags it and publishes the release.

## Code layout

* `src/layout.rs`: geometry, meaning which edges touch and where the cursor
  goes (no Windows dependencies, covered by tests).
* `src/edid.rs`: EDID parsing (physical size and model).
* `src/config.rs`: settings (`%APPDATA%\beezl\config.toml`).
* `src/sys/`: shared Windows code: monitors, autostart through Task Scheduler.
* `src/main.rs`, `src/win/`: the background process `beezl.exe` (mouse hook
  and tray icon).
* `src/ui/`, `ui/app.slint`: the `beezl-ui.exe` windows (tray panel and
  settings, [Slint](https://slint.dev), Fluent style, light and dark theme).
* `src/ui/install.rs`: install and uninstall (the custom Slint installer).
* `vendor/i-slint-backend-winit`: a copy of Slint's winit backend with a fix
  for a division by zero when Windows reports a 0 Hz monitor refresh rate.

## Similar projects

* [LittleBigMouse](https://github.com/mgth/LittleBigMouse): the same approach
  (physical layout from EDID), C#/C++.
* [MouseUnSnag](https://github.com/MouseUnSnag/MouseUnSnag): only removes
  sticking at the edges.

## Credits

The UI is built with [Slint](https://slint.dev) (Royalty-free 2.0 license).

[![Made with Slint](https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-whitebg.png)](https://slint.dev)

## License

[MIT](LICENSE)
