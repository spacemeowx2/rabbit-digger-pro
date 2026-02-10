# Hysteria v2 (HY2) server + client protocol integration

目标：
- 按照官方文档在本机启动一个 Hysteria v2 服务端（用于联调）
- 在本项目实现 HY2 对应的 client protocol（Rust）
- 全程用 Markdown 跟踪进度与决策（本文件）

参考文档（上游）：
- Server Getting Started: `https://v2.hysteria.network/docs/getting-started/Server/`
- Protocol Spec: `https://v2.hysteria.network/docs/developers/Protocol/`

---

## 计划（会随进度更新）

- [ ] 1. 阅读 HY2 docs 并沉淀要点（进行中）
- [ ] 2. 添加本地 server 配置/脚本并启动验证
- [ ] 3. 实现 HY2 client：QUIC + `/auth` + TCP CONNECT(stream)
- [ ] 4. 实现 HY2 client：UDP(datagram) + Salamander(可选)
- [ ] 5. 接入到 registry/feature，补最小测试与使用说明

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
- `Host`（authority）必须为 `hysteria`。
- 必须带 `Authorization` header（值由服务端配置决定）。
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
