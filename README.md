# beezl

**English** · [Русский](README.ru.md)

Smooth cursor movement between monitors of different size and resolution on
Windows: no jumps and no getting stuck at the edges.

## Install

1. [**Download beezl-setup.exe**](../../releases/latest/download/beezl-setup.exe)
   and run it. If Windows says "Windows protected your PC", click
   **More info → Run anyway**.
2. Click **Install** and allow administrator rights once (so beezl also works
   over admin windows like Task Manager).

beezl lives in the tray and starts with Windows. Uninstall it in
**Settings → Apps**.

## Use

* **Click the tray icon** to turn beezl on or off and see your monitors.
* **Right-click** for **Settings** and **Exit**.
* Arrange your monitors in **Windows Settings → System → Display** the way they
  stand on your desk; beezl follows that layout.

## How it works

Windows joins monitors by pixels, so when two screens have different pixel
density the cursor jumps at the border. beezl reads each monitor's physical
size and moves the cursor to the same physical point on the next screen. It
uses almost no CPU: it only acts when the cursor reaches an edge.

## Code signing

Releases are not code-signed yet, so Windows SmartScreen warns on first run.
Every release is built from this repository's source by GitHub Actions, and
the release notes list the installer's SHA-256 checksum.

**Privacy:** this program will not transfer any information to other networked
systems unless specifically requested by the user or the person installing or
operating it.

---

MIT license · UI made with [Slint](https://slint.dev)

[![Made with Slint](https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-whitebg.png)](https://slint.dev)
