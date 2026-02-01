---
sidebar_position: 3
---

# 配置参考

本页汇总常用配置块的字段说明，方便在编写配置时快速查阅。更完整的字段说明可以结合 JSON Schema 进行补全。

## net 类型

`net` 中每个节点使用 `type` 字段指定具体实现。常见类型如下：

| 类型 | 说明 | 关键字段 |
| --- | --- | --- |
| `shadowsocks` | Shadowsocks 客户端 | `server`, `cipher`, `password`, `udp` |
| `trojan` | Trojan 客户端 | `server`, `sni`, `password`, `udp` |
| `http` | HTTP 代理客户端 | `server`, `port`, `username`, `password` |
| `socks5` | SOCKS5 代理客户端 | `server`, `port`, `username`, `password` |
| `rule` | 规则路由 | `rule`, `lru_cache_size` |
| `local` | 本机直连 | 无 |

> 不同 `net` 的字段以 Schema 生成结果为准，建议在编辑器中开启 Schema 补全。

### rule 规则项

`rule` 类型由若干 `rule` 条目组成，每个条目需要声明匹配条件与目标出口。

| 匹配类型 | 字段 | 说明 |
| --- | --- | --- |
| `domain` | `method`, `domain` | 域名匹配，`method` 支持 `keyword`/`suffix`/`match` |
| `ipcidr` | `ipcidr` | 目标 IP 段匹配 |
| `src_ipcidr` | `ipcidr` | 源 IP 段匹配 |
| `geoip` | `country` | 国家代码匹配 |
| `any` | - | 匹配所有流量 |

示例：

```yaml
net:
  my_rule:
    type: rule
    rule:
      - type: domain
        method: suffix
        domain: example.com
        target: proxy
      - type: any
        target: local
```

## server 类型

`server` 中每个条目定义本地监听的服务。

| 类型 | 说明 | 关键字段 |
| --- | --- | --- |
| `http` | HTTP 代理服务 | `bind`, `net` |
| `socks5` | SOCKS5 代理服务 | `bind`, `net` |
| `http+socks5` | 混合模式 | `bind`, `net` |

示例：

```yaml
server:
  mixed:
    type: http+socks5
    bind: 127.0.0.1:10800
    net: my_rule
```

## import 类型

`import` 用于导入外部配置或订阅。

| 类型 | 说明 | 关键字段 |
| --- | --- | --- |
| `merge` | 合并本地配置 | `source.path` |
| `clash` | 导入 Clash 配置 | `source.poll.url`, `source.poll.interval`, `rule_name` |

示例：

```yaml
import:
  - type: merge
    source:
      path: ./local.yaml
  - type: clash
    source:
      poll:
        url: https://example.com/subscribe.yaml
        interval: 86400
    rule_name: clash_rule
```

## 常用顶级字段

| 字段 | 说明 |
| --- | --- |
| `id` | 配置标识（可选） |
| `net` | 代理节点与链路 |
| `server` | 本地监听服务 |
| `import` | 配置导入 |
