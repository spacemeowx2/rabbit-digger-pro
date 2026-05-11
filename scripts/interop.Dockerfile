FROM rust:1-bookworm

ARG TARGETARCH
ARG ANYTLS_VERSION=0.0.12
ARG HYSTERIA_VERSION=2.8.2
ARG SHADOWSOCKS_RUST_VERSION=1.24.0
ARG TROJAN_VERSION=1.16.0

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates curl unzip jq netcat-openbsd openssl python3 xz-utils \
    && rm -rf /var/lib/apt/lists/*

RUN set -eux; \
    xray_version="$(curl -fsSL https://api.github.com/repos/XTLS/Xray-core/releases/latest | jq -r .tag_name)"; \
    case "${TARGETARCH}" in \
      amd64) xray_arch=64; anytls_arch=linux_amd64; hysteria_arch=linux-amd64; ss_arch=x86_64-unknown-linux-gnu; trojan_arch=linux-amd64 ;; \
      arm64) xray_arch=arm64-v8a; anytls_arch=linux_arm64; hysteria_arch=linux-arm64; ss_arch=aarch64-unknown-linux-gnu; trojan_arch= ;; \
      *) echo "unsupported TARGETARCH=${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    curl -fsSL -o /tmp/xray.zip "https://github.com/XTLS/Xray-core/releases/download/${xray_version}/Xray-linux-${xray_arch}.zip"; \
    unzip -q /tmp/xray.zip -d /tmp/xray; \
    install -m 0755 /tmp/xray/xray /usr/local/bin/xray; \
    rm -rf /tmp/xray /tmp/xray.zip; \
    curl -fsSL -o /tmp/anytls.zip "https://github.com/anytls/anytls-go/releases/download/v${ANYTLS_VERSION}/anytls_${ANYTLS_VERSION}_${anytls_arch}.zip"; \
    unzip -q /tmp/anytls.zip -d /tmp/anytls; \
    install -m 0755 /tmp/anytls/anytls-server /usr/local/bin/anytls-server; \
    if [ -f /tmp/anytls/anytls-client ]; then install -m 0755 /tmp/anytls/anytls-client /usr/local/bin/anytls-client; fi; \
    rm -rf /tmp/anytls /tmp/anytls.zip; \
    curl -fsSL -o /usr/local/bin/hysteria "https://github.com/apernet/hysteria/releases/download/app/v${HYSTERIA_VERSION}/hysteria-${hysteria_arch}"; \
    chmod +x /usr/local/bin/hysteria; \
    curl -fsSL -o /tmp/shadowsocks.tar.xz "https://github.com/shadowsocks/shadowsocks-rust/releases/download/v${SHADOWSOCKS_RUST_VERSION}/shadowsocks-v${SHADOWSOCKS_RUST_VERSION}.${ss_arch}.tar.xz"; \
    tar -xJf /tmp/shadowsocks.tar.xz -C /tmp; \
    install -m 0755 /tmp/ssserver /usr/local/bin/ssserver; \
    install -m 0755 /tmp/sslocal /usr/local/bin/sslocal; \
    rm -f /tmp/shadowsocks.tar.xz /tmp/ssserver /tmp/sslocal; \
    if [ -n "$trojan_arch" ]; then \
      curl -fsSL -o /tmp/trojan.tar.xz "https://github.com/trojan-gfw/trojan/releases/download/v${TROJAN_VERSION}/trojan-${TROJAN_VERSION}-${trojan_arch}.tar.xz"; \
      mkdir -p /tmp/trojan; \
      tar -xJf /tmp/trojan.tar.xz -C /tmp/trojan --strip-components=1; \
      install -m 0755 /tmp/trojan/trojan /usr/local/bin/trojan; \
      rm -rf /tmp/trojan /tmp/trojan.tar.xz; \
    fi

RUN set -eux; \
    ssserver --version; \
    sslocal --version; \
    xray version; \
    hysteria version; \
    if command -v trojan >/dev/null 2>&1; then trojan --version; fi; \
    anytls-server --help >/dev/null || true

WORKDIR /workspace
