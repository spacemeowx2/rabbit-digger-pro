---
sidebar_position: 2
---

# 配置文件格式

`rabbit-digger-pro` 使用 YAML 格式的配置文件。配置文件由三个主要的根级字段组成：

## net

`net` 字段用于配置代理节点和链路。每个代理节点都是一个键值对，其中键是节点名称，值是节点配置。

```yaml
net:
  # Shadowsocks 代理节点
  my_ss:
    type: shadowsocks
    server: example.com:1234
    cipher: aes-256-cfb
    password: password

  # HTTP 代理节点
  http_proxy:
    type: http
    server: 127.0.0.1
    port: 8080
```

## server

`server` 字段用于配置本地服务器，比如 HTTP/SOCKS5 代理服务器。

```yaml
server:
  # 混合模式代理服务器
  mixed:
    type: http+socks5
    bind: 127.0.0.1:1080
    net: my_ss # 使用上面定义的 my_ss 节点

  # HTTP 代理服务器
  http:
    type: http
    bind: 127.0.0.1:8080
    net: local # 使用直连节点
```

## import

`import` 字段用于导入其他配置文件或 Clash 配置。rabbit-digger-pro 会根据 `import` 的顺序依次导入配置文件。

```yaml
import:
  # 导入本地配置文件，合并到当前配置
  - type: merge
    source:
      path: ./local-config.yaml

  # 导入 Clash 配置
  - type: clash
    source:
      poll:
        url: "https://example.com/clash-config.yaml"
        interval: 86400
```

### 完整示例

```yaml
net:
  # Shadowsocks 代理节点
  my_ss:
    type: shadowsocks
    server: example.com:1234
    cipher: aes-256-cfb
    password: password

  # HTTP 代理节点
  http_proxy:
    type: http
    server: 127.0.0.1
    port: 8080

server:
  # 混合模式代理服务器
  mixed:
    type: http+socks5
    bind: 127.0.0.1:1080
    net: my_ss # 使用上面定义的 my_ss 节点

  # HTTP 代理服务器
  http:
    type: http
    bind: 127.0.0.1:8080
    net: local # 使用直连节点

import:
  # 导入本地配置文件，合并到当前配置
  - type: merge
    source:
      path: ./local-config.yaml

  # 导入 Clash 配置
  - type: clash
    source:
      poll:
        url: "https://example.com/clash-config.yaml"
        interval: 86400
```
