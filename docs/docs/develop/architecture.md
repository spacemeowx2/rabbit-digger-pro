# 核心概念

Rabbit-Digger Pro (RDP) 是一个通用的网络工具，它能够以灵活的方式组合多种代理协议。本文将介绍 RDP 的核心概念和基础架构。

## 基础网络抽象

RDP 的核心是一组网络抽象 trait，它们定义在 `rd-interface` crate 中：

### Net trait

`Net` 是最基础的网络抽象，它提供了四个基本能力:

```rust
pub trait INet: Downcast + Unpin + Send + Sync {
    fn provide_tcp_connect(&self) -> Option<&dyn TcpConnect>;
    fn provide_tcp_bind(&self) -> Option<&dyn TcpBind>;
    fn provide_udp_bind(&self) -> Option<&dyn UdpBind>;
    fn provide_lookup_host(&self) -> Option<&dyn LookupHost>;
}
```

这四个能力分别是：
- TCP 连接 - 作为客户端连接到远程服务器
- TCP 监听 - 作为服务器接受连接
- UDP 绑定 - 用于 UDP 通信
- 域名解析 - 提供 DNS 查询功能

### Stream Traits 

RDP 定义了几个关键的流处理 trait：

```rust
#[async_trait]
pub trait ITcpStream: Unpin + Send + Sync {
    async fn peer_addr(&self) -> Result<SocketAddr>;
    async fn local_addr(&self) -> Result<SocketAddr>;
    // poll_read/write/flush/shutdown 用于实现异步 IO
    ...
}

#[async_trait]
pub trait ITcpListener: Unpin + Send + Sync {
    async fn accept(&self) -> Result<(TcpStream, SocketAddr)>;
    async fn local_addr(&self) -> Result<SocketAddr>;
}
```

这些 trait 提供了异步 IO 的基础设施，允许实现各种网络协议。

## 协议实现

每个协议需要实现 `Net` trait 来提供其功能。以 Trojan 协议为例：

```rust
pub struct TrojanNet {
    net: Net,  // 底层网络
    config: TrojanConfig,  // 协议配置
}

#[async_trait]
impl rd_interface::TcpConnect for TrojanNet {
    async fn tcp_connect(&self, ctx: &mut Context, addr: &Address) -> Result<TcpStream> {
        // 实现 Trojan 协议的连接逻辑
    }
}

impl INet for TrojanNet {
    fn provide_tcp_connect(&self) -> Option<&dyn rd_interface::TcpConnect> {
        Some(self)
    }
    // ... 提供其他能力
}
```

## 组合模式

RDP 最强大的特性是它的组合能力。例如：

1. **协议嵌套**: 可以将一个 `Net` 作为另一个 `Net` 的底层网络
   ```rust
   // Socks5 over TLS
   let tls_net = TlsNet::new(local_net, tls_config);
   let socks5_net = Socks5Net::new(tls_net, socks5_config);
   ```

2. **协议混合**: 可以在同一个端口上提供多种协议服务
   ```rust
   // HTTP + Socks5 server
   let mixed = HttpSocks5Server::new(listen_net, upstream_net);
   ```

3. **规则路由**: 可以根据规则选择不同的代理
   ```rust
   let rule_net = RuleNet::new(vec![
       ("*.google.com", proxy_a),
       ("*.github.com", proxy_b),
       ("*", direct),
   ]);
   ```

## 上下文传递

`Context` 在整个网络栈中传递，携带连接的相关信息：
- 来源地址
- 目标地址
- 使用的网络路径
- 自定义元数据

这些信息可用于：
- 连接统计
- 访问控制
- 日志记录
- 调试排错

## 错误处理

RDP 使用 `Result<T, Error>` 处理错误，其中 `Error` 类型包含：
- 网络错误
- 协议错误
- 配置错误
- DNS 解析错误等

每个错误都可以携带上下文信息，便于定位问题。

## 异步支持

RDP 基于 Tokio 提供全异步支持：
- 异步 IO
- 异步 DNS 解析
- Future 取消
- 资源自动清理

这使得 RDP 能够高效处理大量并发连接。