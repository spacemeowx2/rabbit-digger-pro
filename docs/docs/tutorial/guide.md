---
sidebar_position: 4
---

# 进阶使用指南

本页整理了一套较完整的实践流程，帮助你从「能跑」快速过渡到「稳定可维护」。所有示例都基于 YAML 配置格式。

## 1. 准备配置文件

建议从一个最小配置开始，确认服务可用后再逐步扩展。

```yaml
net:
  ss_main:
    type: shadowsocks
    server: example.com:1234
    cipher: aes-256-cfb
    password: password
    udp: true
server:
  mixed:
    type: http+socks5
    bind: 127.0.0.1:10800
    net: ss_main
```

## 2. 启动与热重载

默认情况下程序会读取当前目录的 `config.yaml`：

```bash
./rabbit-digger-pro
```

配置文件更新后会自动热重载。建议先在日志中确认「已重载」的提示，再进行网络请求测试。

如果配置不在当前目录，可以显式指定：

```bash
./rabbit-digger-pro -c /path/to/config.yaml
```

## 3. 组合多个出口

使用 `rule` 类型可以对不同目标走不同出口，下面示例展示域名分流：

```yaml
net:
  us:
    type: trojan
    server: us.example.com:443
    sni: us.example.com
    password: "uspassword"
    udp: true
  jp:
    type: shadowsocks
    server: jp.example.com:1234
    cipher: aes-256-cfb
    password: "jppassword"
    udp: true
  my_rule:
    type: rule
    rule:
      - type: domain
        method: suffix
        domain: google.com
        target: jp
      - type: domain
        method: keyword
        domain: twitter
        target: us
      - type: any
        target: local
server:
  mixed:
    type: http+socks5
    bind: 0.0.0.0:10800
    net: my_rule
```

## 4. 订阅与合并配置

如果有 Clash 订阅，可以在 `import` 中拉取并合并：

```yaml
import:
  - type: clash
    poll:
      url: https://example.com/subscribe.yaml
      interval: 86400
    rule_name: clash_rule
```

将合并后的规则在其他位置直接引用：

```yaml
server:
  mixed:
    type: http+socks5
    bind: 127.0.0.1:10800
    net: clash_rule
```

## 5. 使用 JSON Schema 做配置补全

生成 Schema 文件后可用于编辑器补全：

```bash
./rabbit-digger-pro generate-schema > rabbit-digger-pro-schema.json
```

编辑器中引用远程 Schema 也可获得补全（示例）：

```yaml
# yaml-language-server: $schema=https://rabbit-digger.github.io/schema/rabbit-digger-pro-schema.json
```

## 6. 常见排查思路

1. **检查日志**：确认配置已成功加载。
2. **确认端口监听**：例如 `10800` 端口是否已开启。
3. **逐步缩减配置**：从最小配置排查出问题配置块。
4. **检查订阅内容**：确认订阅 YAML 可在浏览器打开。

> 更多参数说明与字段参考请查阅「配置参考」页面。
