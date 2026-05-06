#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RDP_BIN="${RDP_BIN:-$ROOT_DIR/target/debug/rabbit-digger-pro}"
PROTOCOL="${1:-${PROTOCOL:-all}}"
TMP_DIR="$(mktemp -d /tmp/rdp-protocol-interop.XXXXXX)"
PIDS=()

cleanup() {
  local status="$?"
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

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing required command: $1" >&2
    exit 1
  }
}

pick_port() {
  python3 - <<'PY'
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
}

wait_port() {
  local port="$1"
  for _ in $(seq 1 200); do
    if nc -z 127.0.0.1 "$port" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
  done
  echo "port did not open: $port" >&2
  return 1
}

wait_http_ok() {
  local port="$1"
  for _ in $(seq 1 200); do
    if curl -fsS --max-time 2 "http://127.0.0.1:$port/small.txt" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
  done
  echo "http test server did not become ready: $port" >&2
  return 1
}

start_bg() {
  local log_file="$1"
  shift
  "$@" >"$log_file" 2>&1 &
  local pid="$!"
  PIDS+=("$pid")
  echo "$pid"
}

stop_bg() {
  local pid="$1"
  kill "$pid" >/dev/null 2>&1 || true
  wait "$pid" >/dev/null 2>&1 || true
}

curl_via_socks() {
  local proxy_port="$1"
  local out="$2"
  curl -fsS --max-time 20 --proxy "socks5h://127.0.0.1:$proxy_port" \
    "http://127.0.0.1:$HTTP_PORT/small.txt" >"$out"
  cmp -s "$TMP_DIR/www/small.txt" "$out"
}

make_rdp_client_config() {
  local type="$1" server_port="$2" socks_port="$3" extra="$4"
  cat >"$TMP_DIR/rdp-${type}-client.yaml" <<EOF_CFG
id: rdp-${type}-client
net:
  upstream:
    type: ${type}
    server: 127.0.0.1:${server_port}
${extra}
server:
  mixed:
    type: http+socks5
    bind: 127.0.0.1:${socks_port}
    net: upstream
EOF_CFG
}

make_rdp_server_config() {
  local type="$1" listen_port="$2" extra="$3"
  cat >"$TMP_DIR/rdp-${type}-server.yaml" <<EOF_CFG
id: rdp-${type}-server
net: {}
server:
  inbound:
    type: ${type}
    bind: 127.0.0.1:${listen_port}
${extra}
EOF_CFG
}

run_shadowsocks() {
  need_cmd ssserver
  need_cmd sslocal
  local server_port rdp_socks_port rdp_server_port official_socks_port
  server_port="$(pick_port)"
  rdp_socks_port="$(pick_port)"
  rdp_server_port="$(pick_port)"
  official_socks_port="$(pick_port)"

  echo "shadowsocks scenario 1: RDP client -> official server"
  local ss_server_pid
  ss_server_pid="$(start_bg "$TMP_DIR/ss-official-server.log" ssserver \
    -s "127.0.0.1:${server_port}" -m aes-128-gcm -k testpass)"
  wait_port "$server_port"
  make_rdp_client_config shadowsocks "$server_port" "$rdp_socks_port" "    password: testpass
    cipher: aes-128-gcm"
  local rdp_client_pid
  rdp_client_pid="$(start_bg "$TMP_DIR/ss-rdp-client.log" "$RDP_BIN" -c "$TMP_DIR/rdp-shadowsocks-client.yaml")"
  wait_port "$rdp_socks_port"
  curl_via_socks "$rdp_socks_port" "$TMP_DIR/ss-scenario1.txt"
  stop_bg "$rdp_client_pid"
  stop_bg "$ss_server_pid"

  echo "shadowsocks scenario 2: official client -> RDP server"
  make_rdp_server_config shadowsocks "$rdp_server_port" "    password: testpass
    cipher: aes-128-gcm"
  local rdp_server_pid official_client_pid
  rdp_server_pid="$(start_bg "$TMP_DIR/ss-rdp-server.log" "$RDP_BIN" -c "$TMP_DIR/rdp-shadowsocks-server.yaml")"
  wait_port "$rdp_server_port"
  official_client_pid="$(start_bg "$TMP_DIR/ss-official-client.log" sslocal \
    -b "127.0.0.1:${official_socks_port}" -s "127.0.0.1:${rdp_server_port}" -m aes-128-gcm -k testpass)"
  wait_port "$official_socks_port"
  curl_via_socks "$official_socks_port" "$TMP_DIR/ss-scenario2.txt"
  stop_bg "$official_client_pid"
  stop_bg "$rdp_server_pid"
}

run_trojan() {
  need_cmd trojan
  local trojan_server_port rdp_socks_port
  trojan_server_port="$(pick_port)"
  rdp_socks_port="$(pick_port)"

  openssl req -x509 -newkey rsa:2048 -nodes -days 1 \
    -keyout "$TMP_DIR/trojan.key" -out "$TMP_DIR/trojan.crt" -subj '/CN=localhost' >/dev/null 2>&1

  cat >"$TMP_DIR/trojan-server.json" <<EOF_CFG
{
  "run_type": "server",
  "local_addr": "127.0.0.1",
  "local_port": ${trojan_server_port},
  "remote_addr": "127.0.0.1",
  "remote_port": ${HTTP_PORT},
  "password": ["testpass"],
  "log_level": 1,
  "ssl": {
    "cert": "${TMP_DIR}/trojan.crt",
    "key": "${TMP_DIR}/trojan.key"
  }
}
EOF_CFG

  echo "trojan scenario 1: RDP client -> official server"
  local trojan_server_pid rdp_client_pid
  trojan_server_pid="$(start_bg "$TMP_DIR/trojan-official-server.log" trojan "$TMP_DIR/trojan-server.json")"
  wait_port "$trojan_server_port"
  make_rdp_client_config trojan "$trojan_server_port" "$rdp_socks_port" "    password: testpass
    sni: localhost
    skip_cert_verify: true"
  rdp_client_pid="$(start_bg "$TMP_DIR/trojan-rdp-client.log" "$RDP_BIN" -c "$TMP_DIR/rdp-trojan-client.yaml")"
  wait_port "$rdp_socks_port"
  curl_via_socks "$rdp_socks_port" "$TMP_DIR/trojan-scenario1.txt"
  stop_bg "$rdp_client_pid"
  stop_bg "$trojan_server_pid"
}

run_anytls() {
  need_cmd anytls-server
  local server_port rdp_socks_port
  server_port="$(pick_port)"
  rdp_socks_port="$(pick_port)"

  echo "anytls scenario 1: RDP client -> official server"
  local anytls_server_pid
  anytls_server_pid="$(start_bg "$TMP_DIR/anytls-official-server.log" anytls-server \
    -l "127.0.0.1:${server_port}" -p testpass)"
  wait_port "$server_port"
  make_rdp_client_config anytls "$server_port" "$rdp_socks_port" "    password: testpass
    sni: localhost
    skip_cert_verify: true"
  local rdp_client_pid
  rdp_client_pid="$(start_bg "$TMP_DIR/anytls-rdp-client.log" "$RDP_BIN" -c "$TMP_DIR/rdp-anytls-client.yaml")"
  wait_port "$rdp_socks_port"
  curl_via_socks "$rdp_socks_port" "$TMP_DIR/anytls-scenario1.txt"
  stop_bg "$rdp_client_pid"
  stop_bg "$anytls_server_pid"
}

run_vless() {
  need_cmd xray
  local xray_server_port rdp_socks_port rdp_server_port official_socks_port
  xray_server_port="$(pick_port)"
  rdp_socks_port="$(pick_port)"
  rdp_server_port="$(pick_port)"
  official_socks_port="$(pick_port)"

  local x25519_output private_key public_key
  x25519_output="$(xray x25519)"
  private_key="$(printf '%s\n' "$x25519_output" | sed -n 's/^PrivateKey: //p')"
  public_key="$(printf '%s\n' "$x25519_output" | sed -n 's/^Password (PublicKey): //p')"
  test -n "$private_key"
  test -n "$public_key"

  cat >"$TMP_DIR/xray-vless-server.json" <<EOF_CFG
{
  "log": { "loglevel": "warning" },
  "inbounds": [{
    "listen": "127.0.0.1",
    "port": ${xray_server_port},
    "protocol": "vless",
    "settings": { "clients": [{ "id": "27848739-7e61-4ea0-ba56-d8edf2587d12", "flow": "xtls-rprx-vision" }], "decryption": "none" },
    "streamSettings": {
      "network": "tcp",
      "security": "reality",
      "realitySettings": {
        "dest": "www.microsoft.com:443",
        "serverNames": ["www.microsoft.com"],
        "privateKey": "${private_key}",
        "shortIds": ["00000000"]
      }
    }
  }],
  "outbounds": [{ "protocol": "freedom" }]
}
EOF_CFG

  cat >"$TMP_DIR/rdp-vless-client.yaml" <<EOF_CFG
id: rdp-vless-client
net:
  upstream:
    type: vless
    server: 127.0.0.1:${xray_server_port}
    id: 27848739-7e61-4ea0-ba56-d8edf2587d12
    flow: xtls-rprx-vision
    sni: www.microsoft.com
    client_fingerprint: chrome
    reality_public_key: ${public_key}
    reality_short_id: "00000000"
server:
  mixed:
    type: http+socks5
    bind: 127.0.0.1:${rdp_socks_port}
    net: upstream
EOF_CFG

  echo "vless scenario 1: RDP client -> official Xray server"
  local xray_server_pid rdp_client_pid
  xray_server_pid="$(start_bg "$TMP_DIR/vless-xray-server.log" xray run -c "$TMP_DIR/xray-vless-server.json")"
  wait_port "$xray_server_port"
  rdp_client_pid="$(start_bg "$TMP_DIR/vless-rdp-client.log" "$RDP_BIN" -c "$TMP_DIR/rdp-vless-client.yaml")"
  wait_port "$rdp_socks_port"
  curl_via_socks "$rdp_socks_port" "$TMP_DIR/vless-scenario1.txt"
  stop_bg "$rdp_client_pid"
  stop_bg "$xray_server_pid"

  echo "vless scenario 2: official Xray client -> RDP server"
  make_rdp_server_config vless "$rdp_server_port" "    id: 27848739-7e61-4ea0-ba56-d8edf2587d12
    flow: xtls-rprx-vision
    reality_server_name: www.microsoft.com
    reality_private_key: ${private_key}
    reality_short_id: \"00000000\""
  cat >"$TMP_DIR/xray-vless-client.json" <<EOF_CFG
{
  "log": { "loglevel": "warning" },
  "inbounds": [{ "listen": "127.0.0.1", "port": ${official_socks_port}, "protocol": "socks", "settings": { "udp": false } }],
  "outbounds": [{
    "protocol": "vless",
    "settings": { "vnext": [{ "address": "127.0.0.1", "port": ${rdp_server_port}, "users": [{ "id": "27848739-7e61-4ea0-ba56-d8edf2587d12", "encryption": "none", "flow": "xtls-rprx-vision" }] }] },
    "streamSettings": { "network": "tcp", "security": "reality", "realitySettings": { "serverName": "www.microsoft.com", "fingerprint": "chrome", "publicKey": "${public_key}", "shortId": "00000000" } }
  }]
}
EOF_CFG
  local rdp_server_pid xray_client_pid
  rdp_server_pid="$(start_bg "$TMP_DIR/vless-rdp-server.log" "$RDP_BIN" -c "$TMP_DIR/rdp-vless-server.yaml")"
  wait_port "$rdp_server_port"
  xray_client_pid="$(start_bg "$TMP_DIR/vless-xray-client.log" xray run -c "$TMP_DIR/xray-vless-client.json")"
  wait_port "$official_socks_port"
  curl_via_socks "$official_socks_port" "$TMP_DIR/vless-scenario2.txt"
  stop_bg "$xray_client_pid"
  stop_bg "$rdp_server_pid"
}

run_hysteria() {
  need_cmd hysteria
  local official_server_port rdp_socks_port rdp_server_port official_socks_port
  official_server_port="$(pick_port)"
  rdp_socks_port="$(pick_port)"
  rdp_server_port="$(pick_port)"
  official_socks_port="$(pick_port)"

  openssl req -x509 -newkey rsa:2048 -nodes -days 1 \
    -keyout "$TMP_DIR/server.key" -out "$TMP_DIR/server.crt" -subj '/CN=localhost' >/dev/null 2>&1

  cat >"$TMP_DIR/hysteria-official-server.yaml" <<EOF_CFG
listen: 127.0.0.1:${official_server_port}
tls:
  cert: ${TMP_DIR}/server.crt
  key: ${TMP_DIR}/server.key
auth:
  type: password
  password: testpass
EOF_CFG

  echo "hysteria scenario 1: RDP client -> official server"
  local official_server_pid rdp_client_pid
  official_server_pid="$(start_bg "$TMP_DIR/hysteria-official-server.log" hysteria server -c "$TMP_DIR/hysteria-official-server.yaml")"
  sleep 1
  make_rdp_client_config hysteria "$official_server_port" "$rdp_socks_port" "    auth: testpass
    server_name: localhost
    ca_pem: |
$(sed 's/^/      /' "$TMP_DIR/server.crt")"
  rdp_client_pid="$(start_bg "$TMP_DIR/hysteria-rdp-client.log" "$RDP_BIN" -c "$TMP_DIR/rdp-hysteria-client.yaml")"
  wait_port "$rdp_socks_port"
  curl_via_socks "$rdp_socks_port" "$TMP_DIR/hysteria-scenario1.txt"
  stop_bg "$rdp_client_pid"
  stop_bg "$official_server_pid"

  echo "hysteria scenario 2: official client -> RDP server"
  make_rdp_server_config hysteria "$rdp_server_port" "    tls_cert: ${TMP_DIR}/server.crt
    tls_key: ${TMP_DIR}/server.key
    auth: testpass"
  cat >"$TMP_DIR/hysteria-official-client.yaml" <<EOF_CFG
server: 127.0.0.1:${rdp_server_port}
auth: testpass
tls:
  sni: localhost
  insecure: true
socks5:
  listen: 127.0.0.1:${official_socks_port}
EOF_CFG
  local rdp_server_pid official_client_pid
  rdp_server_pid="$(start_bg "$TMP_DIR/hysteria-rdp-server.log" "$RDP_BIN" -c "$TMP_DIR/rdp-hysteria-server.yaml")"
  sleep 1
  official_client_pid="$(start_bg "$TMP_DIR/hysteria-official-client.log" hysteria client -c "$TMP_DIR/hysteria-official-client.yaml")"
  wait_port "$official_socks_port"
  curl_via_socks "$official_socks_port" "$TMP_DIR/hysteria-scenario2.txt"
  stop_bg "$official_client_pid"
  stop_bg "$rdp_server_pid"
}

need_cmd cargo
need_cmd curl
need_cmd nc
need_cmd python3
need_cmd openssl

HTTP_PORT="$(pick_port)"
mkdir -p "$TMP_DIR/www"
printf 'rabbit-digger-pro protocol interop payload\n' >"$TMP_DIR/www/small.txt"
http_pid="$(start_bg "$TMP_DIR/http.log" python3 -m http.server "$HTTP_PORT" --bind 127.0.0.1 --directory "$TMP_DIR/www")"
wait_http_ok "$HTTP_PORT"

echo "building rabbit-digger-pro..."
cargo build --quiet --bin rabbit-digger-pro --manifest-path "$ROOT_DIR/Cargo.toml"

case "$PROTOCOL" in
  all)
    run_shadowsocks
    run_trojan
    run_vless
    run_hysteria
    run_anytls
    ;;
  shadowsocks|ss) run_shadowsocks ;;
  trojan) run_trojan ;;
  vless) run_vless ;;
  hysteria) run_hysteria ;;
  anytls) run_anytls ;;
  *) echo "unknown protocol: $PROTOCOL" >&2; exit 2 ;;
esac

stop_bg "$http_pid"
echo "protocol interop passed: $PROTOCOL"
