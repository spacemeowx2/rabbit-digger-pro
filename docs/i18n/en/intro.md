---
sidebar_position: 1
---

# Introduction

`rabbit-digger` is a proxy software written in [Rust](https://www.rust-lang.org/).


It is still in the ~~rapid~~ *slowly* development stage. The documentation may not be consistent with actual usage, so please [submit an issue](https://github.com/rabbit-digger/rabbit-digger.github.io/issues/new) if you find any inconsistencies.

## Supported Protocol

* Shadowsocks
* Trojan
* HTTP
* Socks5
* obfs(http_simple)

## Supported Server Protocol

* Socks5
* HTTP
* http+socks on the same port
* Shadowsocks


# Installation

Go to the [Release page](https://github.com/rabbit-digger/rabbit-digger-pro/releases) to download the binary file.

# Common Usage

## Normal mode

```
rabbit-digger-pro -c config.example.yaml
```

## Normal mode + Control port + Access Token

```
rabbit-digger-pro -c config.example.yaml -b 127.0.0.1:8030 --access-token token
```

## Control mode, without any config at launch

```
rabbit-digger-pro server -b 127.0.0.1:8030 --access-token token
```

# Command line parameters

```
rabbit-digger-pro 0.1.0

USAGE:
    rabbit-digger-pro [OPTIONS] [SUBCOMMAND]

FLAGS:
    -h, --help       Prints help information
    -V, --version    Prints version information

OPTIONS:
        --access-token <access-token>    Access token [env: RD_ACCESS_TOKEN=]
    -b, --bind <bind>                    HTTP endpoint bind address [env: RD_BIND=]
    -c, --config <config>                Path to config file [env: RD_CONFIG=]  [default: config.yaml]
        --userdata <userdata>            Userdata [env: RD_USERDATA=]
        --web-ui <web-ui>                Web UI. Folder path [env: RD_WEB_UI=]
        --write-config <write-config>    Write generated config to path

SUBCOMMANDS:
    generate-schema    Generate schema to path, if not present, output to stdout
    help               Prints this message or the help of the given subcommand(s)
    server             Run in server mode
```
