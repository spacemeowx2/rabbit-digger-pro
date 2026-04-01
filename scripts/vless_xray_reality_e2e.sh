#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RDP_BIN="${RDP_BIN:-$ROOT_DIR/target/debug/rabbit-digger-pro}"
E2E_URL="${E2E_URL:-https://example.com}"
STRESS_CONCURRENCY="${STRESS_CONCURRENCY:-16}"
LONG_TRANSFER_KIB="${LONG_TRANSFER_KIB:-2048}"
LONG_RATE_LIMIT="${LONG_RATE_LIMIT:-256k}"

if [[ -n "${XRAY_BIN:-}" ]]; then
  XRAY="$XRAY_BIN"
elif [[ -x /opt/homebrew/opt/xray/bin/xray ]]; then
  XRAY=/opt/homebrew/opt/xray/bin/xray
elif [[ -x /usr/local/opt/xray/bin/xray ]]; then
  XRAY=/usr/local/opt/xray/bin/xray
else
  echo "xray binary not found. Set XRAY_BIN or install via Homebrew." >&2
  exit 1
fi

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing required command: $1" >&2
    exit 1
  }
}

need_cmd cargo
need_cmd curl
need_cmd nc
need_cmd python3

TMP_DIR="$(mktemp -d /tmp/rdp-vless-e2e.XXXXXX)"
PIDS=()

cleanup() {
  local status="$?"
  local pid
  for pid in "${PIDS[@]:-}"; do
    kill "$pid" >/dev/null 2>&1 || true
    wait "$pid" >/dev/null 2>&1 || true
  done
  if [[ "${KEEP_TMP:-0}" == "1" || "$status" -ne 0 ]]; then
    echo "preserved logs and configs: $TMP_DIR" >&2
  else
    rm -rf "$TMP_DIR"
  fi
}
trap cleanup EXIT

wait_port() {
  local port="$1"
  local i
  for i in $(seq 1 100); do
    if nc -z 127.0.0.1 "$port" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}

start_bg() {
  local log_file="$1"
  shift
  "$@" >"$log_file" 2>&1 &
  PIDS+=("$!")
  echo "$!"
}

stop_bg() {
  local pid="$1"
  kill "$pid" >/dev/null 2>&1 || true
  wait "$pid" >/dev/null 2>&1 || true
}

wait_http_ok() {
  local port="$1"
  local i
  for i in $(seq 1 100); do
    if curl -fsS --max-time 2 "http://127.0.0.1:$port/small.txt" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}

curl_via_proxy() {
  local proxy_port="$1"
  shift
  curl -sS --proxy "socks5h://127.0.0.1:$proxy_port" "$@"
}

run_concurrent_fetches() {
  local label="$1"
  local proxy_port="$2"
  local expected="$TMP_DIR/www/small.txt"
  local pids=()
  local i

  echo "$label: $STRESS_CONCURRENCY concurrent fetches"
  for i in $(seq 1 "$STRESS_CONCURRENCY"); do
    curl_via_proxy "$proxy_port" --max-time 30 \
      "http://127.0.0.1:$HTTP_PORT/small.txt?i=$i" >"$TMP_DIR/$label-small-$i.txt" &
    pids+=("$!")
  done

  for i in $(seq 1 "$STRESS_CONCURRENCY"); do
    wait "${pids[$((i - 1))]}"
    cmp -s "$expected" "$TMP_DIR/$label-small-$i.txt"
  done
}

run_long_fetch() {
  local label="$1"
  local proxy_port="$2"
  local expected="$TMP_DIR/www/large.bin"
  local output="$TMP_DIR/$label-large.bin"

  echo "$label: long transfer ${LONG_TRANSFER_KIB} KiB at $LONG_RATE_LIMIT"
  curl_via_proxy "$proxy_port" --max-time 120 --limit-rate "$LONG_RATE_LIMIT" \
    "http://127.0.0.1:$HTTP_PORT/large.bin" >"$output"
  cmp -s "$expected" "$output"
}

x25519_output="$("$XRAY" x25519)"
private_key="$(printf '%s\n' "$x25519_output" | awk -F': ' '/^PrivateKey:/ {print $2}')"
public_key="$(printf '%s\n' "$x25519_output" | awk -F': ' '/^Password \(PublicKey\):/ {print $2}')"

if [[ -z "$private_key" || -z "$public_key" ]]; then
  echo "failed to parse xray x25519 output" >&2
  exit 1
fi

HTTP_PORT="$(
  python3 - <<'PY'
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
)"

mkdir -p "$TMP_DIR/www"
cat >"$TMP_DIR/www/small.txt" <<'EOF'
rabbit-digger-pro vless reality stress test payload
EOF
python3 - <<PY
from pathlib import Path
size = int("${LONG_TRANSFER_KIB}") * 1024
payload = (b"rabbit-digger-pro-vless-reality-" * ((size // 32) + 1))[:size]
Path("$TMP_DIR/www/large.bin").write_bytes(payload)
PY
http_pid="$(start_bg "$TMP_DIR/http.log" python3 -m http.server "$HTTP_PORT" --bind 127.0.0.1 --directory "$TMP_DIR/www")"
wait_http_ok "$HTTP_PORT"

cat >"$TMP_DIR/xray-reality-server.json" <<EOF
{
  "log": { "loglevel": "warning" },
  "inbounds": [
    {
      "listen": "127.0.0.1",
      "port": 15443,
      "protocol": "vless",
      "settings": {
        "clients": [
          {
            "id": "27848739-7e61-4ea0-ba56-d8edf2587d12",
            "flow": "xtls-rprx-vision"
          }
        ],
        "decryption": "none"
      },
      "streamSettings": {
        "network": "tcp",
        "security": "reality",
        "realitySettings": {
          "show": false,
          "dest": "www.microsoft.com:443",
          "xver": 0,
          "serverNames": ["www.microsoft.com"],
          "privateKey": "$private_key",
          "shortIds": ["00000000"]
        }
      }
    }
  ],
  "outbounds": [
    { "protocol": "freedom" }
  ]
}
EOF

cat >"$TMP_DIR/rdp-reality-client.yaml" <<EOF
id: rdp-reality-client
net:
  xray-reality:
    type: vless
    server: 127.0.0.1:15443
    id: 27848739-7e61-4ea0-ba56-d8edf2587d12
    flow: xtls-rprx-vision
    sni: www.microsoft.com
    udp: true
    client_fingerprint: chrome
    reality_public_key: $public_key
    reality_short_id: "00000000"
server:
  mixed:
    type: http+socks5
    bind: 127.0.0.1:10888
    net: xray-reality
EOF

cat >"$TMP_DIR/rdp-reality-server.yaml" <<EOF
id: rdp-reality-server
net: {}
server:
  vless:
    type: vless
    bind: 127.0.0.1:16443
    id: 27848739-7e61-4ea0-ba56-d8edf2587d12
    flow: xtls-rprx-vision
    reality_server_name: www.microsoft.com
    reality_private_key: $private_key
    reality_short_id: "00000000"
    udp: true
EOF

cat >"$TMP_DIR/xray-reality-client.json" <<EOF
{
  "log": { "loglevel": "warning" },
  "inbounds": [
    {
      "listen": "127.0.0.1",
      "port": 10890,
      "protocol": "socks",
      "settings": { "udp": true }
    }
  ],
  "outbounds": [
    {
      "protocol": "vless",
      "settings": {
        "vnext": [
          {
            "address": "127.0.0.1",
            "port": 16443,
            "users": [
              {
                "id": "27848739-7e61-4ea0-ba56-d8edf2587d12",
                "encryption": "none",
                "flow": "xtls-rprx-vision"
              }
            ]
          }
        ]
      },
      "streamSettings": {
        "network": "tcp",
        "security": "reality",
        "realitySettings": {
          "serverName": "www.microsoft.com",
          "fingerprint": "chrome",
          "publicKey": "$public_key",
          "shortId": "00000000"
        }
      }
    }
  ]
}
EOF

echo "building rabbit-digger-pro..."
cargo build --quiet --bin rabbit-digger-pro --manifest-path "$ROOT_DIR/Cargo.toml"

echo "scenario 1: curl -> rdp reality client -> xray reality server -> internet"
xray_server_pid="$(start_bg "$TMP_DIR/xray-reality-server.log" "$XRAY" run -c "$TMP_DIR/xray-reality-server.json")"
rdp_client_pid="$(start_bg "$TMP_DIR/rdp-reality-client.log" "$RDP_BIN" -c "$TMP_DIR/rdp-reality-client.yaml")"
wait_port 15443
wait_port 10888
curl -sS --proxy socks5h://127.0.0.1:10888 --max-time 30 -I "$E2E_URL" >"$TMP_DIR/scenario1.headers"
grep -q '^HTTP/' "$TMP_DIR/scenario1.headers"
run_concurrent_fetches "scenario1" 10888
run_long_fetch "scenario1" 10888
stop_bg "$rdp_client_pid"
stop_bg "$xray_server_pid"

echo "scenario 2: curl -> xray reality client -> rdp reality server -> internet"
rdp_server_pid="$(start_bg "$TMP_DIR/rdp-reality-server.log" "$RDP_BIN" -c "$TMP_DIR/rdp-reality-server.yaml")"
xray_client_pid="$(start_bg "$TMP_DIR/xray-reality-client.log" "$XRAY" run -c "$TMP_DIR/xray-reality-client.json")"
wait_port 16443
wait_port 10890
curl -sS --proxy socks5h://127.0.0.1:10890 --max-time 30 -I "$E2E_URL" >"$TMP_DIR/scenario2.headers"
grep -q '^HTTP/' "$TMP_DIR/scenario2.headers"
run_concurrent_fetches "scenario2" 10890
run_long_fetch "scenario2" 10890
stop_bg "$xray_client_pid"
stop_bg "$rdp_server_pid"
stop_bg "$http_pid"

echo "scenario 1 headers:"
sed -n '1,5p' "$TMP_DIR/scenario1.headers"
echo "scenario 2 headers:"
sed -n '1,5p' "$TMP_DIR/scenario2.headers"
echo "set KEEP_TMP=1 to preserve logs and generated configs"
