---
sidebar_position: 3
---

# 快速上手

`Rabbit Digger Pro` 是一个命令行程序. 在不提供参数的情况下, 它会读取工作目录下的 `config.yaml` 作为配置文件, 然后开始运行. 如果需要指定配置文件为其他位置, 可以传入参数 `-c`.

```shell
./rabbit-digger-pro -c config.yaml
```

## 单 Shadowsocks 配置

这个配置文件会在本机监听 `10800` 端口, 并且将传入的代理请求通过 `Shadowsocks` 协议转发到代理服务器.

其中, `example.com:1234` 是远程服务器的地址. 成功运行后, 本机的 `10800` 端口可以接受 `HTTP` 协议和 `SOCKS 5` 协议的代理请求.

`ss_net` 和 `mixed` 可以替换成任意字符串, 它们分别代表这个代理和服务的名字.

如果你修改了 `net` 中的 `ss_net`, 别忘了同时修改 `server` / `mixed` / `net` 中的 `ss_net`.

`config.yaml`:

```yaml
net:
  ss_net:
    type: shadowsocks
    server: example.com:1234
    cipher: aes-256-cfb
    password: password
    udp: true
server:
  mixed:
    type: http+socks5
    bind: 127.0.0.1:10800
    net: ss_net
```

## Clash 订阅

`Rabbit Digger Pro` 支持部分 `Clash` 规则的导入.

在这个样例中, `Rabbit Digger Pro` 会从 `url` 中读取规则和代理, 并将其加入 `net`.
`Clash` 中的所有代理会以相同的名字导入这个配置文件中, 而规则会以 `clash_rule` 命名.

在配置文件的其他地方, 你可以使用由 `import` 导入的代理和规则. 例如在 `server` 中引用 `clash_rule`.

::: warning
请注意, `import` 阶段对 `url` 的请求并不会经过 `Rabbit Digger Pro` 中的任何代理. 如果有通过代理访问的需求, 需要设置环境变量 `http_proxy`, `https_proxy`.
:::

```yaml
server:
  mixed:
    type: http+socks5
    bind: 127.0.0.1:10800
    net: clash_rule
import:
  - type: clash
    poll:
      # Clash 配置地址
      url: https://example.com/subscribe.yaml
      # 每过 86400 秒, 也就是 1 天更新一次
      interval: 86400
    # 生成的规则名
    rule_name: clash_rule
```

如果你的 `Clash` 文件是本地文件, 可以将 `import` 字段改为如下配置:

```yaml
import:
  - type: clash
    path: /path/to/subscribe.yaml
    rule_name: clash_rule
```

## 带规则的多出口代理

在这个样例中, 假设你有 `us`, `jp` 两个出口, `us` 是 `trojan` 协议, `jp` 是 `shadowsocks` 协议.

我们希望在连接发生时, 通过判断域名来走不同的出口:

- 当域名以 `google.com` `结尾`时, 通过 `jp` 连接.
- 当域名中`包含` `twitter` 时, 通过 `us` 连接.
- 其他情况, 通过 `local` 连接.

::: tip
`local` 代表使用本机直接连接. 即使你没有在 `net` 中声明也默认存在. 然而你还是可以通过在 `net` 中声明 `local` 来覆盖这个默认行为.
:::

```yaml
# yaml-language-server: $schema=https://rabbit-digger.github.io/schema/rabbit-digger-pro-schema.json
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
