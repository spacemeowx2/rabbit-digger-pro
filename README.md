# rabbit-digger-pro

![logo](https://user-images.githubusercontent.com/8019167/219358254-dd507c1e-99af-4a70-9081-59e44794edc2.png)

[![codecov][codecov-badge]][codecov-url]
[![MIT licensed][mit-badge]][mit-url]
[![Build Status][actions-badge]][actions-url]

[codecov-badge]: https://codecov.io/gh/rabbit-digger/rabbit-digger-pro/branch/main/graph/badge.svg?token=VM9N0IGMWE
[mit-badge]: https://img.shields.io/badge/license-MIT-blue.svg
[actions-badge]: https://github.com/rabbit-digger/rabbit-digger-pro/workflows/Build/badge.svg

[codecov-url]: https://codecov.io/gh/rabbit-digger/rabbit-digger-pro
[mit-url]: https://github.com/rabbit-digger/rabbit-digger-pro/blob/master/LICENSE
[actions-url]: https://github.com/rabbit-digger/rabbit-digger-pro/actions?query=workflow%3ABuild+branch%3Amain

All-in-one proxy written in Rust.

## Features

* Hot reloading: Apply changes without restart the program.
* Flexible configuration: proxies can be nested at will, supporting TCP and UDP.
* JSON Schema generation: no documentation needed, write configuration directly from code completion.

### Supported Protocol

* Shadowsocks
* Trojan
* Hysteria v2 (HY2)
* VLESS (`xtls-rprx-vision`, TLS/REALITY)
* HTTP
* Socks5
* obfs(http_simple)

### Supported Server Protocol

* Socks5
* HTTP
* http+socks5 on the same port
* Shadowsocks
* VLESS (`xtls-rprx-vision`, TLS/REALITY)

## Interop Check

Run `bash scripts/vless_xray_reality_e2e.sh` to validate real Xray REALITY interoperability with public HTTP traffic via `curl`. The script uses `XRAY_BIN` if set, otherwise falls back to the Homebrew `xray` install path.
By default it also runs `16` concurrent proxied fetches and one rate-limited `2 MiB` long-lived transfer in each direction. Tune them with `STRESS_CONCURRENCY`, `LONG_TRANSFER_KIB`, and `LONG_RATE_LIMIT`.
For a longer soak, for example, run `STRESS_CONCURRENCY=32 LONG_TRANSFER_KIB=8192 LONG_RATE_LIMIT=128k bash scripts/vless_xray_reality_e2e.sh`.
For GitHub Actions, use the manual workflow [vless-xray-interop.yml](/Users/space/project/rabbit-digger-pro/.github/workflows/vless-xray-interop.yml) on macOS to run the same real-binary checks.

## crates

* rd-derive

Used to conveniently define the Config structure.

* rd-std

Some basic net and server, such as rule, HTTP and Socks5.

* rd-interface

Interface defines of rabbit-digger's plugin.

## Credits

* [shadowsocks-rust](https://github1s.com/shadowsocks/shadowsocks-rust)
* [smoltcp](https://github.com/smoltcp-rs/smoltcp)
