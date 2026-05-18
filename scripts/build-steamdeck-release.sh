#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="${RDP_VERSION:-$(grep -m1 '^version =' "$ROOT/Cargo.toml" | sed -E 's/version = "([^"]+)"/\1/')}"
CHANNEL="${RDP_CHANNEL:-dev}"
OUT_DIR="${RDP_OUT_DIR:-$ROOT/dist/steamdeck}"
RELEASE_BASE_URL="${RDP_RELEASE_BASE_URL:-https://github.com/spacemeowx2/rabbit-digger-pro/releases/download/v$VERSION}"
PLUGIN_DIR="$ROOT/packaging/decky/rabbit-digger-pro"
HELPER_NAME="rabbit-digger-pro-x86_64-unknown-linux-gnu"
PLUGIN_ZIP="rabbit-digger-pro-decky.zip"
MANIFEST_NAME="steamdeck-update-manifest.json"

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

need_cmd cargo
need_cmd pnpm
need_cmd python3
need_cmd zip

if [ "$(uname -s)" != "Linux" ] && [ "${RDP_ALLOW_NON_LINUX:-0}" != "1" ]; then
  echo "Steam Deck release assets must be built on Linux. Set RDP_ALLOW_NON_LINUX=1 for local smoke builds." >&2
  exit 1
fi

rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"

cargo build --release --features webui_embed
cp "$ROOT/target/release/rabbit-digger-pro" "$OUT_DIR/$HELPER_NAME"

(
  cd "$PLUGIN_DIR"
  pnpm install --frozen-lockfile=false
  pnpm run build
)

PACKAGE_ROOT="$OUT_DIR/plugin-package"
rm -rf "$PACKAGE_ROOT"
mkdir -p "$PACKAGE_ROOT/rabbit-digger-pro"
cp -R "$PLUGIN_DIR/dist" "$PACKAGE_ROOT/rabbit-digger-pro/dist"
cp "$PLUGIN_DIR/main.py" "$PACKAGE_ROOT/rabbit-digger-pro/main.py"
cp "$PLUGIN_DIR/plugin.json" "$PACKAGE_ROOT/rabbit-digger-pro/plugin.json"
python3 - "$PLUGIN_DIR/package.json" "$PACKAGE_ROOT/rabbit-digger-pro/package.json" "$VERSION" <<'PY'
import json
import sys
from pathlib import Path

source, dest, version = sys.argv[1:]
package = json.loads(Path(source).read_text(encoding="utf-8"))
package["version"] = version
Path(dest).write_text(json.dumps(package, indent=2) + "\n", encoding="utf-8")
PY

(
  cd "$PACKAGE_ROOT"
  zip -qr "$OUT_DIR/$PLUGIN_ZIP" rabbit-digger-pro
)
rm -rf "$PACKAGE_ROOT"

HELPER_SHA="$(sha256_file "$OUT_DIR/$HELPER_NAME")"
PLUGIN_SHA="$(sha256_file "$OUT_DIR/$PLUGIN_ZIP")"

python3 - "$OUT_DIR/$MANIFEST_NAME" "$VERSION" "$CHANNEL" "$RELEASE_BASE_URL" "$HELPER_NAME" "$HELPER_SHA" "$PLUGIN_ZIP" "$PLUGIN_SHA" <<'PY'
import datetime
import json
import sys
from pathlib import Path

path, version, channel, base, helper_name, helper_sha, plugin_zip, plugin_sha = sys.argv[1:]
manifest = {
    "version": version,
    "channel": channel,
    "published_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    "helper": {
        "url": f"{base}/{helper_name}",
        "sha256": helper_sha,
    },
    "decky_plugin": {
        "url": f"{base}/{plugin_zip}",
        "sha256": plugin_sha,
    },
}
Path(path).write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
PY

echo "Steam Deck release assets written to $OUT_DIR"
ls -lh "$OUT_DIR"
