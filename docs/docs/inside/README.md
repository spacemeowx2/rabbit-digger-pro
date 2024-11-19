---
sidebar: auto
---

# Inside

`rabbit-digger` 内部的秘密 🐰...

## Net

`Net` 是 `rabbit-digger` 的核心概念. 每个代理协议都是根据一个配置(如代理服务器地址, 认证方式, 基于的 `Net`) 构造一个新的 `Net`. 这个 `Net` 提供了 `tcp_connect` 和 `udp_bind`, 对使用者隐藏了服务器的连接细节, 能够让使用者直接调用 `tcp_connect` 和 `udp_bind`.

`Net` 的实现者不应该使用异步运行时提供的 `TcpStream` 和 `UdpSocket` 来连接代理服务器. 而是应该在 `Config` 中声明 `NetRef`, 然后使用这个 `Net` 来连接代理服务器.

因此, 每个代理协议都能够互相嵌套, 自然的实现了代理链.

## NetRef

`NetRef` 是一个 `enum`, 有 `String` 和 `Net` 两种状态. 当 `Config` 从文件读入时, `NetRef` 是一个未解析的字符串. 而 `rabbit-digger` 会根据引用关系一次将 `NetRef` 解析成 `Net` 实例, 然后传给 `NetFactory::new`.

## ExternalFile

`ExternalFile` 可用在 `Config` 中. 代表着这个字段是一个外部的文件. `ExternalFile` 可以是文件, 也可以是 `Url`. 当 `ExternalFile` 是文件且 `watch` 为 `true` 时, `Net` 会在文件变更时被重建. 当 `Url` 和 `interval` 被设置时, 文件会被轮询, 并且在改变时重建 `Net`.

## Config 处理流

所有 `Config` 类型都实现了 `Config` trait, `rabbit-digger` 会在加载 `Net` 时调用 `Config::visit` 来访问内部的字段, 并填入所有的 `NetRef`, `ExternalFile`. 在填入 `ExternalFile` 的时候会记录所有使用到的文件, 并在文件变动的时候重新构建 `Net`.

```mermaid
flowchart TD
    input(Config.yml)
    mkctx[创建配置上下文, 用于保存 Config 依赖的文件]
    import[处理 Import 字段]
    build[构造 Net 和 Server]
    cond{依赖的文件
    是否改变?}
    run_server[运行 Server]

    input --> mkctx --> import --> build --> run_server --> cond
    cond --> |Yes| mkctx
    cond --> |No| run_server
```
