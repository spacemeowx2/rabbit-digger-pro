# 插件开发指南

本文介绍如何为 Rabbit-Digger Pro 开发新的功能和协议支持。

## 插件类型

RDP 支持以下类型的插件:

1. 协议插件 - 实现新的代理协议
2. 网络插件 - 提供特殊的网络功能
3. 服务器插件 - 提供新的服务端功能
4. 中间件插件 - 处理或修改网络流量

## 开发新协议

### 1. 创建新的 Crate

首先创建一个新的 Rust crate，添加必要的依赖：

```toml
[package]
name = "rd-plugin-myprotocol"
version = "0.1.0"

[dependencies]
rd-interface = { path = "../rd-interface" }
rd-derive = { path = "../rd-derive" }
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"
```

### 2. 定义配置结构

使用 `rd_config` 宏定义配置结构：

```rust
use rd_derive::rd_config;

#[rd_config]
#[derive(Debug)]
pub struct MyProtocolConfig {
    server: String,
    password: String,
    // 其他配置字段
}
```

### 3. 实现核心功能

创建协议的主要结构体并实现必要的 trait：

```rust
pub struct MyProtocol {
    net: Net,
    config: MyProtocolConfig,
}

#[async_trait]
impl rd_interface::TcpConnect for MyProtocol {
    async fn tcp_connect(
        &self,
        ctx: &mut Context,
        addr: &Address
    ) -> Result<TcpStream> {
        // 实现协议的连接逻辑
    }
}

impl INet for MyProtocol {
    fn provide_tcp_connect(&self) -> Option<&dyn rd_interface::TcpConnect> {
        Some(self)
    }
}
```

### 4. 注册插件

在 `lib.rs` 中注册插件：

```rust
use rd_interface::registry::Builder;

impl Builder<Net> for MyProtocol {
    const NAME: &'static str = "myprotocol";
    type Config = MyProtocolConfig;
    type Item = Self;

    fn build(net: Net, config: Self::Config) -> Result<Self> {
        Ok(MyProtocol { net, config })
    }
}
```

## 开发实用工具

### 1. TCP Stream 包装

如果需要修改或检查 TCP 流量，可以包装 `TcpStream`：

```rust
pub struct MyTcpStream {
    inner: TcpStream,
    // 其他字段
}

#[async_trait]
impl ITcpStream for MyTcpStream {
    async fn peer_addr(&self) -> Result<SocketAddr> {
        self.inner.peer_addr().await
    }
    
    async fn local_addr(&self) -> Result<SocketAddr> {
        self.inner.local_addr().await
    }

    fn poll_read(
        &mut self,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        // 可以在这里处理读取的数据
        self.inner.poll_read(cx, buf)
    }

    fn poll_write(
        &mut self,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        // 可以在这里处理写入的数据
        self.inner.poll_write(cx, buf)
    }
}
```

### 2. 实现中间件

中间件可以用来修改或监控网络流量：

```rust
pub struct MyMiddleware {
    inner: Net,
}

#[async_trait]
impl rd_interface::TcpConnect for MyMiddleware {
    async fn tcp_connect(
        &self,
        ctx: &mut Context,
        addr: &Address,
    ) -> Result<TcpStream> {
        // 在连接前后添加处理逻辑
        let stream = self.inner.tcp_connect(ctx, addr).await?;
        Ok(MyTcpStream::new(stream).into_dyn())
    }
}
```

## 最佳实践

1. **错误处理**
   - 使用 `rd_interface::Error` 表示错误
   - 为自定义错误实现 `std::error::Error`
   - 提供详细的错误上下文

```rust
use rd_interface::Error;

fn handle_error(err: impl std::error::Error) -> Error {
    Error::Other(Box::new(err))
}
```

2. **异步编程**
   - 使用 `async/await` 语法
   - 避免阻塞操作
   - 正确实现 Future 取消

3. **配置验证**
   - 在构建时验证配置
   - 提供合理的默认值
   - 使用 `#[validate]` 属性

4. **资源管理**
   - 实现 `Drop` trait 清理资源
   - 使用 `Arc` 共享资源
   - 避免循环引用

## 示例项目

这里是一个完整的示例项目结构：

```
rd-plugin-example/
├── Cargo.toml
├── src/
│   ├── lib.rs           # 插件注册
│   ├── config.rs        # 配置定义
│   ├── protocol.rs      # 协议实现
│   ├── stream.rs        # 流处理
│   └── error.rs         # 错误处理
└── tests/
    └── integration.rs   # 集成测试
```

## 调试技巧

1. **日志输出**
```rust
use tracing::{info, warn, error};

async fn handle_connection() {
    info!("New connection");
    if let Err(e) = do_something().await {
        error!("Connection error: {}", e);
    }
}
```

2. **使用 Context**
```rust
ctx.insert("my_key", "value")?;
if let Some(value) = ctx.get("my_key") {
    // 使用值
}
```

3. **性能分析**
```rust
use tracing::instrument;

#[instrument]
async fn my_function() {
    // 函数执行会被自动记录
}
```