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
- 可选 “Salamander” 对 QUIC UDP 包做混淆：每个 UDP 包前加 8 字节 `salt`，计算 `BLAKE2b-256(key || salt)` 得到 32 字节 key stream，对 payload 做 XOR；发送为 `salt || xor(payload)`，接收反向处理。

### 认证（HTTP/3 `/auth`）
- QUIC 建连后，client 必须发送 HTTP/3 `POST /auth`，并要求 `:host` 为 `hysteria`（本项目 client 发送 `https://hysteria/auth`）。
- 认证 header（规范）：`Hysteria-Auth: <string>`。
- 兼容（非规范但常见）：`Authorization: <password>` / `Authorization: Bearer <password>`（本项目 server 兼容三种）。
- 成功响应 HTTP status 为 `233`，并返回：
  - `Hysteria-UDP: true/false`
  - `Hysteria-CC-RX: <uint|auto>`（本项目 server 默认返回 `auto`）
  - `Hysteria-Padding: <string>`（随机 padding string，双方应忽略）
- 可选请求 header：
  - `Hysteria-CC-RX: <uint>`（bytes/s；0 表示未知）
  - `Hysteria-Padding: <string>`（随机 padding string，双方应忽略）
- 响应 header（用于能力判断）：
  - `Hysteria-UDP`: `true/false`
  - `Hysteria-CC-RX`: server 接受的下行 bytes/s（或 `auto`）

### TCP（双向 QUIC stream）
- 每次 TCP 连接：打开一个双向 QUIC stream，写入 `TCPRequest`：
  - `varint(0x401)` (TCPRequest ID)
  - `varint(Address length)` + `Address string (host:port)`
  - `varint(Padding length)` + `Random padding`
- server 必须先回 `TCPResponse`：
  - `uint8(Status)`：`0x00=OK`，`0x01=Error`
  - `varint(Message length)` + `Message string`
  - `varint(Padding length)` + `Random padding`
- 只有在 response OK 后，双方才开始转发 TCP payload。

### UDP（QUIC datagram）
- UDP 包通过 QUIC unreliable datagram 发送，格式 `UDPMessage`（双向一致）：
  - `uint32 Session ID`
  - `uint16 Packet ID`
  - `uint8 Fragment ID`
  - `uint8 Fragment Count`
  - `varint Address length` + `Address string (host:port)`
  - `Payload bytes`
- 分片要求：超过 `max_datagram_size` 必须分片或丢弃；丢任一分片则整包丢弃。

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
- 2026-02-11：对齐官方 Protocol Spec：`Hysteria-Auth`/`TCPResponse`/`UDPMessage` 统一格式、Salamander(8-byte salt)
- 2026-02-11：增强健壮性与测试：H3 drain、UDP session idle 回收、分片重组 TTL、codec/salamander/auth/interop 扩展测试
- 2026-02-11：与上游 `hysteria` v2.7.0 互通验证通过：官方 client → 本项目 server；本项目 client → 官方 server（TCP/HTTP 经 SOCKS5 验证）

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

当前实现已按官方 Protocol Spec（Hysteria v2 / “v4” 协议）对齐并自测通过，覆盖：
- QUIC 传输 + HTTP/3 `/auth`：`Hysteria-Auth`/`Hysteria-CC-RX`/`Hysteria-Padding`（client/server）
- TCP：`TCPRequest(0x401)` + `TCPResponse` + 双向转发（client/server）
- UDP：`UDPMessage`（双向同格式）+ 分片重组（client/server）
- Salamander：8-byte salt + `BLAKE2b-256(key || salt)`（client/server）
- 单测：真实端口联调（TCP echo + UDP echo）`protocol/hysteria/src/interop_tests.rs`

仍未覆盖/可能影响“对抗探测/更强伪装/更复杂边界”的点：
- HTTP/3 masquerade：当前只返回 404（未实现静态站点或反代上游站点）。
- 拥塞控制/带宽协商：`Hysteria-CC-RX` 目前按协议交互（client 发 uint；server 回 `auto`），未实现额外的应用层限速策略。
- 生命周期细节：auth 后 server 侧已保持 H3 连接持续驱动并对后续请求返回 404；更复杂的“伪装站点/反代”仍未实现。
