#!/usr/bin/env bash
set -euo pipefail

DEFAULT_MANIFEST_URL="https://github.com/spacemeowx2/rabbit-digger-pro/releases/latest/download/steamdeck-update-manifest.json"
DEFAULT_DECKY_INSTALLER_URL="https://github.com/SteamDeckHomebrew/decky-installer/releases/latest/download/install_release.sh"
MANIFEST_URL="${RDP_MANIFEST_URL:-$DEFAULT_MANIFEST_URL}"
DECKY_INSTALLER_URL="${RDP_DECKY_INSTALLER_URL:-$DEFAULT_DECKY_INSTALLER_URL}"
SKIP_DECKY_INSTALL="${RDP_SKIP_DECKY_INSTALL:-0}"
REQUIRE_DECKY="${RDP_REQUIRE_DECKY:-0}"
BIND="${RDP_BIND:-127.0.0.1:9091}"
COMMAND="${1:-install}"

DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
RDP_DATA_DIR="$DATA_HOME/rabbit_digger_pro"
RDP_CONFIG_DIR="$CONFIG_HOME/rabbit_digger_pro"
UPDATE_CONFIG_PATH="$RDP_CONFIG_DIR/update.json"
PLUGIN_DIR="$HOME/homebrew/plugins/rabbit-digger-pro"
HELPER_DIR="$RDP_DATA_DIR/helper"
HELPER_PATH="$HELPER_DIR/rabbit-digger-pro"
USER_UNIT_PATH="$CONFIG_HOME/systemd/user/rabbit-digger-pro.service"
TMP_DIR=""

usage() {
  cat <<'EOF'
Usage:
  scripts/install-steamdeck.sh [install|update]
  scripts/install-steamdeck.sh uninstall

Environment:
  RDP_MANIFEST_URL         Override the update manifest URL.
  RDP_BIND                 Override helper bind address. Default: 127.0.0.1:9091.
  RDP_SKIP_DECKY_INSTALL   Set to 1 to skip the Decky Loader auto-install attempt.
  RDP_REQUIRE_DECKY        Set to 1 to fail if Decky Loader is missing.
EOF
}

cleanup_tmp() {
  if [ -n "${TMP_DIR:-}" ]; then
    rm -rf "$TMP_DIR"
  fi
}

trap cleanup_tmp EXIT

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

json_get() {
  python3 - "$1" "$2" <<'PY'
import json
import sys

path, dotted = sys.argv[1], sys.argv[2]
value = json.load(open(path, "r", encoding="utf-8"))
for part in dotted.split("."):
    value = value[part]
print(value)
PY
}

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

download_verified() {
  local url="$1"
  local expected="$2"
  local dest="$3"

  curl -fL "$url" -o "$dest"
  local actual
  actual="$(sha256_file "$dest")"
  if [ "$actual" != "$expected" ]; then
    echo "Checksum mismatch for $url" >&2
    echo "expected: $expected" >&2
    echo "actual:   $actual" >&2
    exit 1
  fi
}

decky_installed() {
  if systemctl is-active --quiet plugin_loader 2>/dev/null; then
    return 0
  fi
  if systemctl is-enabled --quiet plugin_loader 2>/dev/null; then
    return 0
  fi
  if [ -x "$HOME/homebrew/services/PluginLoader" ]; then
    return 0
  fi
  return 1
}

install_decky_loader() {
  if [ "$SKIP_DECKY_INSTALL" = "1" ]; then
    echo "Decky Loader auto-install skipped."
    return 1
  fi

  if ! curl -fsI --connect-timeout 8 --max-time 20 https://github.com >/dev/null; then
    echo "Decky Loader was not detected, and GitHub is not reachable from this Steam Deck."
    echo "Continuing without Decky Loader; the Rabbit Digger Pro plugin files will still be staged."
    return 1
  fi

  echo "Decky Loader was not detected. Installing Decky Loader from the official installer..."
  local installer="$TMP_DIR/decky-install-release.sh"
  if ! curl -fL --connect-timeout 8 --max-time 60 "$DECKY_INSTALLER_URL" -o "$installer"; then
    echo "Failed to download the Decky Loader installer."
    return 1
  fi
  if ! sh "$installer"; then
    echo "Decky Loader installer failed."
    return 1
  fi

  if ! decky_installed; then
    echo "Decky Loader installation did not complete or could not be detected." >&2
    return 1
  fi

  return 0
}

restart_decky_loader_if_running() {
  if systemctl is-active --quiet plugin_loader 2>/dev/null; then
    systemctl restart plugin_loader 2>/dev/null || true
  fi
}

fallback_uninstall_user_service() {
  systemctl --user stop rabbit-digger-pro.service 2>/dev/null || true
  systemctl --user disable rabbit-digger-pro.service 2>/dev/null || true
  rm -f "$USER_UNIT_PATH"
  systemctl --user daemon-reload 2>/dev/null || true
  rm -f "$HELPER_PATH"
}

install_or_update() {
  need_cmd curl
  need_cmd python3
  need_cmd systemctl
  need_cmd unzip

  mkdir -p "$RDP_DATA_DIR" "$RDP_CONFIG_DIR"

  TMP_DIR="$(mktemp -d)"
  DECKY_READY=0

  if decky_installed; then
    DECKY_READY=1
  elif install_decky_loader; then
    DECKY_READY=1
  elif [ "$REQUIRE_DECKY" = "1" ]; then
    echo "Decky Loader is required but is not installed." >&2
    exit 1
  fi

  MANIFEST_PATH="$TMP_DIR/manifest.json"
  curl -fL "$MANIFEST_URL" -o "$MANIFEST_PATH"

  HELPER_URL="$(json_get "$MANIFEST_PATH" helper.url)"
  HELPER_SHA="$(json_get "$MANIFEST_PATH" helper.sha256)"
  PLUGIN_URL="$(json_get "$MANIFEST_PATH" decky_plugin.url)"
  PLUGIN_SHA="$(json_get "$MANIFEST_PATH" decky_plugin.sha256)"

  HELPER_TMP="$TMP_DIR/rabbit-digger-pro"
  PLUGIN_TMP="$TMP_DIR/rabbit-digger-pro-decky.zip"

  download_verified "$HELPER_URL" "$HELPER_SHA" "$HELPER_TMP"
  chmod +x "$HELPER_TMP"

  "$HELPER_TMP" service install-user \
    --bind "$BIND" \
    --binary "$HELPER_TMP"

  download_verified "$PLUGIN_URL" "$PLUGIN_SHA" "$PLUGIN_TMP"
  rm -rf "$TMP_DIR/plugin"
  mkdir -p "$TMP_DIR/plugin"
  unzip -q "$PLUGIN_TMP" -d "$TMP_DIR/plugin"

  rm -rf "$PLUGIN_DIR"
  mkdir -p "$(dirname "$PLUGIN_DIR")"
  PLUGIN_ROOT="$(find "$TMP_DIR/plugin" -mindepth 1 -maxdepth 1 -type d | head -n 1)"
  if [ -z "$PLUGIN_ROOT" ]; then
    echo "Plugin zip did not contain a plugin directory" >&2
    exit 1
  fi
  cp -R "$PLUGIN_ROOT" "$PLUGIN_DIR"

  restart_decky_loader_if_running

  python3 - "$UPDATE_CONFIG_PATH" "$MANIFEST_URL" "$HELPER_PATH" "$PLUGIN_DIR" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
path.parent.mkdir(parents=True, exist_ok=True)
path.write_text(json.dumps({
    "manifest_url": sys.argv[2],
    "components": [
        {
            "source": "helper",
            "kind": "file",
            "path": sys.argv[3],
            "executable": True,
        },
        {
            "source": "decky_plugin",
            "kind": "zip-dir",
            "path": sys.argv[4],
        },
    ],
}, indent=2), encoding="utf-8")
PY

  echo "Rabbit Digger Pro installed."
  if [ "$DECKY_READY" = "1" ]; then
    echo "Return to Gaming Mode and open Decky Loader."
  else
    echo "Decky Loader is not installed yet, so the Gaming Mode menu will not appear."
    echo "The helper and plugin files are staged; install Decky Loader later and restart it."
  fi
}

uninstall_rdp() {
  need_cmd systemctl

  echo "Uninstalling Rabbit Digger Pro..."

  if [ -x "$HELPER_PATH" ]; then
    if ! "$HELPER_PATH" service uninstall-user; then
      echo "Helper uninstall failed; continuing with fallback cleanup." >&2
      fallback_uninstall_user_service
    fi
  else
    fallback_uninstall_user_service
  fi

  rm -rf "$PLUGIN_DIR"
  rm -f "$UPDATE_CONFIG_PATH" "$RDP_CONFIG_DIR/decky-update.json"
  rmdir "$HELPER_DIR" "$RDP_DATA_DIR" "$RDP_CONFIG_DIR" 2>/dev/null || true
  restart_decky_loader_if_running

  echo "Rabbit Digger Pro removed."
  echo "Decky Loader was left installed because other plugins may depend on it."
}

case "$COMMAND" in
  install | update)
    shift || true
    if [ "$#" -ne 0 ]; then
      usage >&2
      exit 1
    fi
    install_or_update
    ;;
  uninstall | remove | clean | --uninstall)
    shift || true
    if [ "$#" -ne 0 ]; then
      usage >&2
      exit 1
    fi
    uninstall_rdp
    ;;
  -h | --help | help)
    usage
    ;;
  *)
    usage >&2
    exit 1
    ;;
esac
