#!/usr/bin/env bash
#
# install.sh — First-time installer, desktop integrator & periodic updater setup
# for RepoHarbor on Linux and macOS.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/azmykn/RepoHarbor/main/scripts/install.sh | bash
#   bash scripts/install.sh [--from-source] [--autostart] [--with-auto-update]
#   bash scripts/install.sh [--uninstall] [--purge]
#
# Installs:
#   - Binary:  ~/.local/bin/repoharbor (atomic swap — safe while running)
#   - Desktop: ~/.local/share/applications/com.digitscode.repoharbor.desktop (Linux)
#              ~/Applications/RepoHarbor.app (macOS)
#   - Meta:    install-meta.toml in user data dir
#   - Optional: autostart entry on login (--autostart)
#   - Optional: systemd user timer repoharbor-update.timer on Linux (--with-auto-update)
set -euo pipefail

REPO_SLUG="azmykn/RepoHarbor"
APP_ID="com.digitscode.repoharbor"
BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
BIN_DST="$BIN_DIR/repoharbor"

UNAME_S="$(uname -s)"
UNAME_M="$(uname -m)"
case "$UNAME_S" in
  Darwin)
    TARGET_OS="macos"
    DATA_DIR="$HOME/Library/Application Support/repoharbor"
    CFG_DIR="$HOME/Library/Application Support/repoharbor"
    AUTOSTART_FILE="$HOME/Library/LaunchAgents/${APP_ID}.plist"
    ;;
  *)
    TARGET_OS="linux"
    DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/repoharbor"
    CFG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/repoharbor"
    AUTOSTART_FILE="${XDG_CONFIG_HOME:-$HOME/.config}/autostart/${APP_ID}.desktop"
    ;;
esac

case "$UNAME_M" in
  arm64|aarch64) TARGET_ARCH="aarch64" ;;
  *)             TARGET_ARCH="x86_64" ;;
esac

SYSTEMD_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"

MODE="auto"
AUTOSTART=0
WITH_AUTO_UPDATE=0
UNINSTALL=0
PURGE=0
ASSUME_YES=0

for arg in "$@"; do
  case "$arg" in
    --from-source)      MODE="source" ;;
    --from-release)     MODE="release" ;;
    --autostart)        AUTOSTART=1 ;;
    --with-auto-update) WITH_AUTO_UPDATE=1 ;;
    --uninstall)        UNINSTALL=1 ;;
    --purge)            PURGE=1 ;;
    -y|--yes)           ASSUME_YES=1 ;;
    -h|--help)
      sed -n '2,18p' "$0" | sed 's/^# \{0,1\}//'
      exit 0 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

say()  { printf '\n\033[1m▸ %s\033[0m\n' "$*"; }
ok()   { printf '  \033[32m✓\033[0m %s\n' "$*"; }
warn() { printf '  \033[33m!\033[0m %s\n' "$*"; }
have() { command -v "$1" >/dev/null 2>&1; }

SCRIPT_DIR=""
REPO_ROOT=""
if [[ -n "${BASH_SOURCE[0]:-}" && -f "${BASH_SOURCE[0]:-}" ]]; then
  SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  if [[ -f "$SCRIPT_DIR/../Cargo.toml" ]]; then
    REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
  fi
fi

do_uninstall() {
  say "Uninstalling RepoHarbor user installation…"
  if have systemctl; then
    systemctl --user disable --now repoharbor-update.timer >/dev/null 2>&1 || true
  fi
  rm -f "$SYSTEMD_DIR/repoharbor-update.timer" "$SYSTEMD_DIR/repoharbor-update.service"
  rm -f "$AUTOSTART_FILE"
  rm -f "$BIN_DST"
  if [[ "$TARGET_OS" == "macos" ]]; then
    rm -rf "$HOME/Applications/RepoHarbor.app"
  else
    APP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
    ICON_BASE="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor"
    rm -f "$APP_DIR/${APP_ID}.desktop" "$APP_DIR/repoharbor.desktop"
    for size in 32x32 64x64 128x128 256x256 512x512 scalable; do
      rm -f "$ICON_BASE/${size}/apps/repoharbor.png" \
            "$ICON_BASE/${size}/apps/repoharbor.svg" \
            "$ICON_BASE/${size}/apps/${APP_ID}.png" \
            "$ICON_BASE/${size}/apps/${APP_ID}.svg"
    done
  fi
  rm -f "$DATA_DIR/install-meta.toml"
  if [[ "$PURGE" == "1" ]]; then
    rm -rf "$DATA_DIR" "$CFG_DIR"
    ok "removed user data and config directories"
  fi
  ok "RepoHarbor uninstalled"
  exit 0
}

[[ "$UNINSTALL" == "1" ]] && do_uninstall

mkdir -p "$BIN_DIR" "$DATA_DIR"

write_meta() {
  local kind="$1"
  local src_dir="${2:-}"
  local ver="$3"
  local now
  now="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  {
    printf 'kind = "%s"\n' "$kind"
    if [[ -n "$src_dir" ]]; then
      printf 'sourceDir = "%s"\n' "$src_dir"
    fi
    printf 'version = "%s"\n' "$ver"
    printf 'installedAt = "%s"\n' "$now"
  } > "$DATA_DIR/install-meta.toml"
}

install_from_release() {
  say "Checking latest GitHub Release ($REPO_SLUG for ${TARGET_OS}-${TARGET_ARCH})…"
  local api_json
  api_json="$(curl -fsSL -H 'Accept: application/vnd.github+json' \
    "https://api.github.com/repos/${REPO_SLUG}/releases/latest" 2>/dev/null || true)"
  if [[ -z "$api_json" ]]; then
    return 1
  fi
  local tar_url sums_url tag
  tar_url="$(printf '%s' "$api_json" | grep -Eo "\"browser_download_url\":[[:space:]]*\"[^\"]+${TARGET_OS}-${TARGET_ARCH}\.tar\.gz\"" | head -n1 | cut -d'"' -f4 || true)"
  if [[ -z "$tar_url" ]]; then
    tar_url="$(printf '%s' "$api_json" | grep -Eo "\"browser_download_url\":[[:space:]]*\"[^\"]+${TARGET_OS}[^\"]+\.tar\.gz\"" | head -n1 | cut -d'"' -f4 || true)"
  fi
  sums_url="$(printf '%s' "$api_json" | grep -Eo '"browser_download_url":[[:space:]]*"[^"]+SHA256SUMS\.txt"' | head -n1 | cut -d'"' -f4 || true)"
  tag="$(printf '%s' "$api_json" | grep -Eo '"tag_name":[[:space:]]*"[^"]+"' | head -n1 | cut -d'"' -f4 || true)"

  [[ -n "$tar_url" ]] || return 1

  local tmp
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' RETURN

  local tar_name="${tar_url##*/}"
  say "Downloading $tar_name (${tag:-latest})…"
  curl -fL --progress-bar -o "$tmp/$tar_name" "$tar_url"

  if [[ -n "$sums_url" ]]; then
    curl -fsSL -o "$tmp/SHA256SUMS.txt" "$sums_url" || true
    if grep -q "$tar_name" "$tmp/SHA256SUMS.txt" 2>/dev/null; then
      if have sha256sum; then
        (cd "$tmp" && grep "$tar_name" SHA256SUMS.txt | sha256sum -c -)
        ok "SHA-256 checksum verified"
      elif have shasum; then
        (cd "$tmp" && grep "$tar_name" SHA256SUMS.txt | sed 's/ \*/  /' | shasum -a 256 -c -)
        ok "SHA-256 checksum verified"
      fi
    fi
  fi

  tar -xzf "$tmp/$tar_name" -C "$tmp"
  local unpacked_bin
  unpacked_bin="$(find "$tmp" -type f -name repoharbor | head -n1)"
  [[ -n "$unpacked_bin" ]] || { echo "archive missing repoharbor binary" >&2; return 1; }

  mkdir -p "$BIN_DIR"
  cp -f "$unpacked_bin" "$BIN_DST.tmp"
  chmod 755 "$BIN_DST.tmp"
  mv -f "$BIN_DST.tmp" "$BIN_DST"
  if [[ "$TARGET_OS" == "macos" ]] && have xattr; then
    xattr -dr com.apple.quarantine "$BIN_DST" >/dev/null 2>&1 || true
  fi
  ok "installed $BIN_DST"

  local ver="${tag#v}"
  write_meta "binary" "" "${ver:-0.0.0}"

  if [[ "$AUTOSTART" == "1" ]]; then
    "$BIN_DST" install-desktop --autostart
  else
    "$BIN_DST" install-desktop
  fi
  return 0
}

install_from_source() {
  local work_repo="$REPO_ROOT"
  if [[ -z "$work_repo" ]]; then
    have git || { echo "git is required for --from-source" >&2; exit 1; }
    work_repo="$DATA_DIR/src"
    if [[ -d "$work_repo/.git" ]]; then
      say "Updating local source checkout ($work_repo)…"
      git -C "$work_repo" pull --ff-only
    else
      say "Cloning $REPO_SLUG into $work_repo…"
      git clone "https://github.com/${REPO_SLUG}.git" "$work_repo"
    fi
  fi

  cd "$work_repo"
  if [[ "$TARGET_OS" == "linux" ]] && { ! have cargo || [[ ! -f "$HOME/.cargo/env" && ! -x "$(command -v cargo || true)" ]]; }; then
    local setup_flags=()
    [[ "$ASSUME_YES" == "1" ]] && setup_flags+=(-y)
    bash scripts/setup.sh "${setup_flags[@]}"
  fi
  [[ -f "$HOME/.cargo/env" ]] && source "$HOME/.cargo/env"

  say "Building release binary (cargo build --release -p repoharbor)…"
  cargo build --release -p repoharbor

  mkdir -p "$BIN_DIR"
  cp -f "$work_repo/target/release/repoharbor" "$BIN_DST.tmp"
  chmod 755 "$BIN_DST.tmp"
  mv -f "$BIN_DST.tmp" "$BIN_DST"
  ok "installed $BIN_DST cleanly (atomic swap)"

  local ver
  ver="$(grep -m1 '^version' "$work_repo/crates/repoharbor/Cargo.toml" | cut -d'"' -f2 || echo "0.0.0")"
  write_meta "source" "$work_repo" "$ver"

  if [[ "$AUTOSTART" == "1" ]]; then
    "$BIN_DST" install-desktop --autostart
  else
    "$BIN_DST" install-desktop
  fi
}

setup_periodic_update_timer() {
  mkdir -p "$SYSTEMD_DIR"
  cat > "$SYSTEMD_DIR/repoharbor-update.service" <<EOF
[Unit]
Description=RepoHarbor periodic release update check
After=network-online.target

[Service]
Type=oneshot
ExecStart=$BIN_DST update --quiet
EOF

  cat > "$SYSTEMD_DIR/repoharbor-update.timer" <<EOF
[Unit]
Description=Daily RepoHarbor update check

[Timer]
OnCalendar=daily
RandomizedDelaySec=1h
Persistent=true

[Install]
WantedBy=timers.target
EOF

  if have systemctl; then
    systemctl --user daemon-reload >/dev/null 2>&1 || true
    systemctl --user enable --now repoharbor-update.timer >/dev/null 2>&1 || true
    ok "enabled systemd user timer: repoharbor-update.timer (daily)"
  else
    warn "systemd not active; run 'repoharbor update' periodically or use in-app updater"
  fi
}

# ── Execute install ──────────────────────────────────────────────────────────
if [[ "$MODE" == "source" ]]; then
  install_from_source
elif [[ "$MODE" == "release" ]]; then
  install_from_release || {
    echo "no published release found on GitHub; run with --from-source instead" >&2
    exit 1
  }
else
  if ! install_from_release; then
    warn "no published GitHub release asset yet — falling back to local release build…"
    install_from_source
  fi
fi

if [[ "$WITH_AUTO_UPDATE" == "1" ]]; then
  setup_periodic_update_timer
fi

say "Done!"
ok "Version: $("$BIN_DST" --version 2>/dev/null || echo 'installed')"
ok "Launch app: repoharbor  (or from your desktop app launcher)"
ok "Update anytime: repoharbor update  (or click Update inside the app)"
if [[ ":$PATH:" != *":$BIN_DIR:"* ]]; then
  warn "add $BIN_DIR to PATH in ~/.bashrc / ~/.zshrc: export PATH=\"$BIN_DIR:\$PATH\""
fi

# If a RepoHarbor GUI instance is currently running, restart it into the newly installed binary.
if have pgrep; then
  RUNNING_PIDS="$(pgrep -x repoharbor 2>/dev/null | tr '\n' ' ' || true)"
  if [[ -n "${RUNNING_PIDS// /}" ]]; then
    ok "Restarting running RepoHarbor instance (${RUNNING_PIDS% })…"
    RESTART_CMD="sleep 0.2; kill -TERM $RUNNING_PIDS 2>/dev/null || true; sleep 0.35; exec \"$BIN_DST\""
    if have systemd-run && systemd-run --user --collect --unit="repoharbor-restart-$(date +%s)" sh -c "$RESTART_CMD" >/dev/null 2>&1; then
      true
    elif have setsid; then
      setsid sh -c "trap '' HUP; $RESTART_CMD" </dev/null >/dev/null 2>&1 &
    else
      nohup sh -c "trap '' HUP; $RESTART_CMD" </dev/null >/dev/null 2>&1 &
    fi
  fi
fi
