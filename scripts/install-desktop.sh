#!/usr/bin/env bash
# Install a user-local .desktop entry + hicolor icons so RepoHarbor can be pinned
# to the taskbar / dash when run from a source build (cargo run).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${REPOHARBOR_BIN:-$ROOT/target/debug/repoharbor}"
APP_ID="com.digitscode.repoharbor"
NAME="repoharbor"
# Pre-rename ids — leave these in place and GNOME/KDE match the wrong desktop
# file (or none), so "Pin to Dash/Favorites" never appears for RepoHarbor.
LEGACY_IDS=(com.orrery.app orrery)

if [[ ! -x "$BIN" ]]; then
  echo "binary not found: $BIN" >&2
  echo "build first: cargo build -p repoharbor" >&2
  exit 1
fi

APP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICON_BASE="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor"
mkdir -p "$APP_DIR"

remove_legacy() {
  local id
  for id in "${LEGACY_IDS[@]}"; do
    rm -f "$APP_DIR/${id}.desktop"
    # Best-effort icon cleanup across common sizes.
    for size in 32x32 64x64 128x128 256x256 512x512 scalable; do
      rm -f "$ICON_BASE/${size}/apps/${id}.png" \
            "$ICON_BASE/${size}/apps/${id}.svg"
    done
  done
}

install_icon() {
  local size="$1" src="$2"
  local dir="$ICON_BASE/${size}/apps"
  mkdir -p "$dir"
  cp -f "$src" "$dir/${NAME}.png"
  # Some shells look up by app id name as well.
  cp -f "$src" "$dir/${APP_ID}.png"
}

remove_legacy

install_icon 32x32   "$ROOT/packaging/icons/32x32.png"
install_icon 64x64   "$ROOT/packaging/icons/64x64.png"
install_icon 128x128 "$ROOT/packaging/icons/128x128.png"
install_icon 512x512 "$ROOT/packaging/icons/icon.png"

if [[ -f "$ROOT/packaging/icons/repoharbor.svg" ]]; then
  mkdir -p "$ICON_BASE/scalable/apps"
  cp -f "$ROOT/packaging/icons/repoharbor.svg" "$ICON_BASE/scalable/apps/${NAME}.svg"
  cp -f "$ROOT/packaging/icons/repoharbor.svg" "$ICON_BASE/scalable/apps/${APP_ID}.svg"
fi

DESKTOP_SRC="$ROOT/packaging/repoharbor.desktop"
DESKTOP_DST="$APP_DIR/${APP_ID}.desktop"
# Also keep a short name for menus that look up "repoharbor".
DESKTOP_ALIAS="$APP_DIR/${NAME}.desktop"

# Escape spaces in the binary path for desktop Exec= (rare, but cheap).
BIN_ESC=${BIN// /\\ }

sed -e "s|^Exec=.*|Exec=${BIN_ESC}|" \
    -e "s|^Icon=.*|Icon=${NAME}|" \
    "$DESKTOP_SRC" > "$DESKTOP_DST"
cp -f "$DESKTOP_DST" "$DESKTOP_ALIAS"

# Refresh caches when tools exist (best-effort).
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$APP_DIR" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -t "$ICON_BASE" >/dev/null 2>&1 || true
fi

echo "Installed:"
echo "  $DESKTOP_DST"
echo "  $DESKTOP_ALIAS"
echo "  icons under $ICON_BASE/*/apps/${NAME}.{png,svg}"
echo "Removed legacy Orrery desktop entries (if any)."
echo
echo "To pin on GNOME/KDE:"
echo "  1. Fully quit any running RepoHarbor/Orrery window"
echo "  2. Launch from the app menu as “RepoHarbor” (or: $BIN)"
echo "  3. Right-click the Dash/taskbar icon → Pin to Dash / Pin to Task Manager"
echo "If Pin is still missing, log out/in once so the shell reloads .desktop files."
echo "StartupWMClass / app_id must be: ${APP_ID}"
