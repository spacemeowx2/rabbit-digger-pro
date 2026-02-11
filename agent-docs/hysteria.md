# Hysteria v2 (HY2) server + client protocol integration

目标：
- 按照官方文档在本机启动一个 Hysteria v2 服务端（用于联调）
- 在本项目实现 HY2 对应的 client protocol（Rust）
- 在本项目实现 HY2 对应的 server protocol（Rust）
- 全程用 Markdown 跟踪进度与决策（本文件）

参考文档（上游）：
- Server Getting Started: `https://v2.hysteria.network/docs/getting-started/Server/`
- Protocol Spec: `https://v2.hysteria.network/docs/developers/Protocol/`

---

## 计划（会随进度更新）

- [x] 1. 阅读 HY2 docs 并沉淀要点
- [x] 2. 添加本地 server 配置/脚本并启动验证
- [x] 3. 实现 HY2 client：QUIC + `/auth` + TCP CONNECT(stream)
- [x] 4. 实现 HY2 client：UDP(datagram) + Salamander(可选)
- [x] 5. 接入到 registry/feature，补最小测试与使用说明
- [x] 6. 实现 HY2 server：QUIC + HTTP/3 `/auth` + TCP/UDP 转发
- [x] 7. 单测：server+client 联调

---

## 协议要点摘录（用于实现）

### 服务端配置（最小可用）
- 官方模板（Own certificate）核心字段：
  - `listen`（可选，默认 `:443`；本地联调会改成非特权端口）
  - `tls.cert` / `tls.key`
  - `auth.type=password` + `auth.password`
  - `masquerade` 可选；不配置时 HTTP 请求会返回 404（不影响 `/auth` 成功与否取决于实现，但联调建议先不加 masquerade，问题更少）

### 传输层（QUIC + 可选 Salamander）
- HY2 基于 QUIC。
- 可选 “Salamander” 对 UDP 包做混淆：每个 UDP 包前加 16 字节 `salt`，计算 `BLAKE2b-256(salt || key)` 得到 32 字节 key stream，对 payload 做 XOR；发送为 `salt || xor(payload)`，接收反向处理。

### 认证（HTTP/3 `/auth`）
- QUIC 建连后，client 必须发送 HTTP 请求到 `/auth`。
- 规范要求 `Host`（authority）为 `hysteria`（本项目 client 发送 `https://hysteria/auth`，满足该要求）。
- 必须带 `Authorization` header（值由服务端配置决定）。实测上游实现存在两种形式：
  - `Authorization: <password>`
  - `Authorization: Bearer <password>`（本项目 server 兼容两者）
- 成功响应的 HTTP status 为 `233`。
- 可选请求 header：
  - `Hysteria-CC-RX`: 下行 bps（不实现复杂 CC，先按文档要求透传/默认）
  - `Hysteria-Padding`: `true/false`
  - `Hysteria-UDP`: `true/false`（是否启用 UDP）
- 响应 header（用于能力判断）：
  - `Hysteria-UDP`: `true/false`
  - `Hysteria-CC-RX`: server 接受的下行 bps

### TCP（双向 QUIC stream）
- 每次 TCP 连接：打开一个双向 QUIC stream，写入 `TCPRequest`：
  - `varint(0x401)` message type
  - `varint(AddressType)`：IPv4=0, Domain=1, IPv6=2
  - address：
    - IPv4: 4 bytes
    - IPv6: 16 bytes
    - Domain: `varint(len)` + `domain bytes`
  - `varint(port)`
- 后续该 stream 就是 TCP payload 双向转发。

### UDP（QUIC datagram）
- client->server: `0x3` + `UDPMessage`（含 address）
- server->client: `0x2` + `UDPMessage`（不含 address）
- `UDPMessage` 字段：
  - `session ID` uint32
  - `packet ID` uint16（每 session 递增）
  - `frag ID` uint8
  - `frag count` uint8
  - `address length` varint + `address bytes`（仅 client->server；address 为 `host:port` 字符串）
  - `payload bytes`

---

## 进度日志

- 2026-02-10：创建跟踪文档，完成协议/配置要点摘录（进行中）
- 2026-02-10：新增本地联调用 server 配置与证书生成脚本：`agent-docs/hysteria/`
- 2026-02-10：新增 `protocol/hysteria`：完成 HY2 `/auth` + TCP stream request 的最小实现（编译/单测通过）
- 2026-02-10：`protocol/hysteria` 增加 UDP(datagram) 与 Salamander(传输层混淆) 支持（编译/单测通过）
- 2026-02-10：本地联调验证：`rabbit-digger-pro` 通过 HY2(TCP) 成功代理访问本机 `python -m http.server`（socks5 -> rdp -> hysteria server -> localhost 目标）
- 2026-02-11：新增 `protocol/hysteria` server（QUIC + H3 `/auth` + TCP/UDP 转发，编译通过）
- 2026-02-11：新增联调单测：`protocol/hysteria/src/interop_tests.rs`（TCP + UDP）
- 2026-02-11：兼容 `Authorization: Bearer <password>`；并将 rustls provider 选择收敛到统一入口（避免运行时 panic）

---

## rabbit-digger-pro 配置示例（本地联调）

前置：先用 `agent-docs/hysteria/README.md` 启动本地 hysteria server。

```yaml
net:
  hy2_local:
    type: hysteria
    server: 127.0.0.1:18443
    server_name: localhost
    auth: test-password
    ca_pem: agent-docs/hysteria/cert.pem
    # udp: true
    # salamander: my-shared-key

server:
  mixed:
    type: http+socks5
    bind: 127.0.0.1:1080
    net: hy2_local
```

---

## 覆盖范围与已知差异（实现完整性说明）

当前实现目标是“能在本项目内自洽运行 + 联调通过”的最小可用子集，已覆盖：
- QUIC 传输 + HTTP/3 `/auth`（client/server）
- TCP：`0x401` 建连消息 + 双向转发（client/server）
- UDP：datagram `0x3/0x2` + 分片重组（client/server）
- Salamander：作为底层 UDP socket 的可选混淆层（client/server）
- 真实端口联调单测：`protocol/hysteria/src/interop_tests.rs`

仍未覆盖/可能与规范或上游实现存在差异的点（后续互通性风险来源）：
- `/auth` 校验：server 目前**不强制** `:authority == hysteria`（规范要求但先放宽兼容），仅校验 path 与 `Authorization`。
- `/auth` header 语义：`Hysteria-CC-RX`、`Hysteria-Padding` 等仅按“能通过握手/不破坏互通”处理，未实现完整的拥塞控制/带宽协商逻辑。
- 连接与生命周期：server 侧 H3 driver 目前是最小保活模型（能跑通测试），对更复杂的并发/长连接场景可能还需打磨。
