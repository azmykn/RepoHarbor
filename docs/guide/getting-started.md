# Getting started

Install RepoHarbor in one command, grab a release package, or build from source.

## Quick install (recommended)

RepoHarbor runs natively on **Linux**, **macOS**, and **Windows** from the same unified Rust + GPUI codebase, and all three platforms receive updates together.

### Linux & macOS

Install into `~/.local/bin/repoharbor` and register the desktop launcher (`.desktop` on Linux, `~/Applications/RepoHarbor.app` on macOS) with zero `sudo`:

```sh
curl -fsSL https://raw.githubusercontent.com/azmykn/RepoHarbor/main/scripts/install.sh | bash
```

Optional flags:

- `--from-source` — build the release binary from source (runs `scripts/setup.sh` first on Linux)
- `--autostart` — launch RepoHarbor automatically when you log in
- `--with-auto-update` — enable a daily `systemd --user` update timer on Linux (`repoharbor-update.timer`)

### Windows (PowerShell)

Install into `%LOCALAPPDATA%\Programs\RepoHarbor\repoharbor.exe`, add it to your user `PATH`, and create a Start Menu shortcut:

```powershell
irm https://raw.githubusercontent.com/azmykn/RepoHarbor/main/scripts/install.ps1 | iex
```

Optional parameters (`scripts\install.ps1`): `-FromSource`, `-Autostart`, `-Uninstall`, `-Purge`.

On first launch, RepoHarbor automatically scans common workspace folders (`~/odoo`, `~/Tools`, `~/dev`, `~/projects`, `~/work`) and surfaces a compact **First-run setup** bar to connect GitHub or pin the app to your desktop launcher.

## Periodic updates

- **In-app:** When a new version lands on GitHub Releases, both an **`Update <tag>`** button in the top header bar and an `↑ vYYYY.M.P` badge in the title bar appear. Click **`Update <tag>`** (or open **Settings → About → Updates & installation** and click **Update & Restart**) for a SHA-256 verified atomic update.
- **CLI:** Run `repoharbor update` anytime (supports `--check` and `--quiet`). Binary installs verify SHA-256 checksums and atomically swap `~/.local/bin/repoharbor` while running; source installs pull and rebuild `--release`.

## Install a release package

Or grab the latest artifact for your distro from the [releases page](https://github.com/azmykn/RepoHarbor/releases):

```sh
# Snap (Ubuntu, Debian, Fedora, Arch, etc.)
sudo snap install --classic repoharbor

# AppImage — portable, no install
chmod +x RepoHarbor_*_amd64.AppImage
./RepoHarbor_*_amd64.AppImage

# Debian / Ubuntu
sudo apt install ./RepoHarbor_*_amd64.deb

# Fedora / RHEL
sudo dnf install ./RepoHarbor-*.x86_64.rpm
```

Release builds ship a bundled `llama.cpp` engine, so the [local-AI](./local-ai) features work out of the box once you download a model — no separate install required.

## Prerequisites

RepoHarbor is a native Rust app on [GPUI](https://www.gpui.rs) — no webview. Building
from source needs:

- **Rust** — the toolchain pinned in `rust-toolchain.toml` (rustup auto-selects it)
- **GPUI system libraries** — Vulkan, Wayland/XCB, `libxkbcommon`, `fontconfig`,
  plus a C/C++ toolchain, `cmake` and `pkg-config`

The repo ships a setup script that installs these across dnf/apt/pacman/zypper:

```sh
bash scripts/setup.sh
```

Or install them by hand. On a Debian/Ubuntu base:

```sh
sudo apt install libvulkan-dev libwayland-dev libxkbcommon-dev \
  libxkbcommon-x11-dev libxcb1-dev libfontconfig1-dev libssl-dev \
  build-essential cmake pkg-config
```

On Fedora:

```sh
sudo dnf install vulkan-loader-devel vulkan-headers mesa-vulkan-drivers \
  wayland-devel libxkbcommon-devel libxkbcommon-x11-devel libxcb-devel \
  fontconfig-devel openssl-devel gcc gcc-c++ make cmake pkgconf-pkg-config
```

> Node + pnpm are **optional** — only the docs site and the icon generator use them.

## Clone and run

```sh
git clone https://github.com/azmykn/RepoHarbor.git
cd RepoHarbor
cargo run -p repoharbor          # first build is slow: it compiles all of GPUI
```

## Pin to the Dash / taskbar

Source builds are not registered with the desktop shell until you install a
user-local `.desktop` entry and icons:

```sh
cargo build -p repoharbor
bash scripts/install-desktop.sh
```

Then quit the app (if it is running), launch **RepoHarbor** from the
applications menu, and right-click its Dash / task manager icon → **Pin to
Dash** (GNOME) or **Pin to Task Manager** (KDE). The window `app_id`
(`com.digitscode.repoharbor`) must match `StartupWMClass` in the desktop file —
the install script wires that up. Relaunch after install; if the Pin option is
still missing, log out and back in once.

On first launch, open **Settings → Workspace roots** and point RepoHarbor at the directories where you keep your projects (defaults to `~/dev`). Then **Save**.

## Build a release

Tagged releases (`v*`) build the bundles in CI. To produce them locally:

```sh
cargo build --release -p repoharbor
cargo install cargo-deb cargo-generate-rpm
cargo deb -p repoharbor --no-build          # → target/debian/*.deb
cargo generate-rpm -p crates/repoharbor     # → target/generate-rpm/*.rpm
bash packaging/appimage.sh              # → dist-appimage/*.AppImage (needs linuxdeploy)
```

## Useful commands

| Command | What it does |
|---|---|
| `cargo run -p repoharbor` | Run the desktop app |
| `cargo build --workspace` | Build everything |
| `cargo test --workspace` | Run the tests |
| `cargo clippy --workspace --all-targets -- -D warnings` | Lint |
| `cargo fmt --all` | Format |
| `bash scripts/dev-clean.sh` | Force an asset re-embed and relaunch |
| `pnpm icons` | Regenerate the committed icon SVGs (optional) |

Next: the [feature tour](./mission-control) or [configuration reference](./configuration).
