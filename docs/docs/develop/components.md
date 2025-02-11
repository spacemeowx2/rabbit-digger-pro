# 网络组件

Rabbit-Digger Pro (RDP) 提供了多种内置的网络组件，每个组件都实现了 `Net` trait 的一个或多个功能。本文将介绍这些组件的用途和配置方式。

## 内置网络组件

### LocalNet

`LocalNet` 是最基础的网络组件，它封装了操作系统提供的网络功能：

```yaml
type: local
bind_addr: 0.0.0.0  # 可选，绑定地址
mark: 1  # 可选，用于策略路由
```

### Socks5Client

实现 Socks5 客户端协议：

```yaml
type: socks5
server: 127.0.0.1:1080
username: optional_user  # 可选
password: optional_pass  # 可选
```

### TlsNet

为底层连接提供 TLS 加密：

```yaml
type: tls
net: local  # 底层网络
sni: example.com  # 可选，指定 SNI
verify: true  # 是否验证证书
```

### RuleNet

根据规则选择不同的网络：

```yaml
type: rule
rules:
  - type: domain  # 域名规则
    domain:
      - "*.google.com"
      - "*.github.com"
    target: proxy
  - type: ip  # IP 规则
    ip:
      - "8.8.8.8/32"
    target: direct
nets:
  proxy: socks5_client  # 引用其他网络配置
  direct: local
```

## 服务器组件

### Socks5Server

提供 Socks5 代理服务：

```yaml
type: socks5_server
listen: 127.0.0.1:1080
net: local  # 出口网络
auth:  # 可选的认证配置
  username: user
  password: pass
```

### HttpServer

提供 HTTP 代理服务：

```yaml
type: http_server
listen: 127.0.0.1:8080
net: local  # 出口网络
auth:  # 可选的认证配置
  username: user
  password: pass
```

### TrojanServer

提供 Trojan 代理服务：

```yaml
type: trojan_server
listen: 0.0.0.0:443
net: local
cert: cert.pem
key: key.pem
password: your_password
```

## 观察和过滤组件

### DNSSniffer

用于嗅探和记录 DNS 查询：

```yaml
type: dns_sniffer
net: local  # 底层网络
```

### SNISniffer

用于嗅探 TLS SNI，实现域名分流：

```yaml
type: sni_sniffer
net: local
ports: [443, 8443]  # 要嗅探的端口
```

## 组件组合示例

1. **通过 Socks5 代理访问 HTTPS**:
```yaml
nets:
  tls_over_socks:
    type: tls
    net: socks5_client
    verify: true
  socks5_client:
    type: socks5
    server: proxy.example.com:1080
```

2. **域名分流代理**:
```yaml
nets:
  main:
    type: rule
    rules:
      - type: domain
        domain: ["*.cn"]
        target: direct
      - type: domain
        domain: ["*.google.com"]
        target: proxy
    nets:
      direct: local
      proxy:
        type: socks5
        server: proxy.example.com:1080
```

## 最佳实践

1. **性能优化**：
   - 对频繁访问的域名使用 DNS 缓存
   - 对大流量服务使用直连或就近的代理
   - 避免不必要的协议封装

2. **安全建议**：
   - 总是为公网服务启用认证
   - 使用 TLS 保护传输安全
   - 定期更新密码和证书

3. **调试技巧**：
   - 使用 SNISniffer 诊断 TLS 连接问题
   - 使用 DNSSniffer 排查域名解析问题
   - 查看详细日志定位连接异常