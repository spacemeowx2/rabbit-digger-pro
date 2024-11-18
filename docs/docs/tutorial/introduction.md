---
sidebar_position: 1
---

# 介绍

`rabbit-digger-pro` 是由 [Rust](https://www.rust-lang.org/) 编写的代理软件.

### 核心特性

- **热重载支持** - 配置文件修改后实时生效，无需重启程序
- **灵活的配置系统** - 支持代理的任意嵌套组合，完整支持 TCP 和 UDP 转发
- **JSON Schema 支持** - 提供配置文件的代码补全功能，无需查阅文档即可编写

### 协议支持

- **多协议兼容** - 支持 Shadowsocks、Trojan、HTTP、SOCKS5 等主流代理协议
- **规则路由系统** - 强大的分流规则引擎，支持域名、IP、GeoIP 等多种匹配方式
- **Clash 配置兼容** - 可直接导入现有的 Clash 配置文件，无缝迁移

### 开发友好

- **API 接口** - 提供 HTTP API 接口，支持程序化控制和状态监控
- **插件系统** - 可扩展的插件架构，支持自定义协议和功能开发
- **跨平台支持** - 支持 Windows、Linux、macOS 等主流操作系统

### 其他亮点

- 配置文件支持 YAML 格式
- 内置 DNS 解析功能，支持 DNS over HTTPS
- 提供详细的连接日志和统计信息
- 低资源占用，性能优异

## 支持的代理协议

- Shadowsocks
- Trojan
- HTTP
- Socks5
- obfs(http_simple)

## 支持的服务器协议

- Socks5
- HTTP
- http+socks on the same port
- Shadowsocks
