---
sidebar_position: 2
---

# 下载安装

前往 [Release 页面](https://github.com/spacemeowx2/rabbit-digger-pro/releases) 下载对应平台的二进制文件.

- Windows 用户: `rabbit-digger-pro-windows-amd64.exe`
- Mac 用户: `rabbit-digger-pro-macos-amd64`
- Linux 用户: `rabbit-digger-pro-x86_64-unknown-linux-musl`

下载后, 将二进制文件放到任意目录下.

本文档之后会使用 `./rabbit-digger-pro` 作为命令调用. 因此可以将文件重命名为 `rabbit-digger-pro`.

## 命令行参数说明

```bash
Usage: rabbit-digger-pro.exe [OPTIONS] [COMMAND]

Commands:
  generate-schema  生成 JSON Schema 配置模板，若未指定路径则输出到标准输出
  server           以服务器模式运行
  help             显示帮助信息

Options:
  -c, --config <CONFIG>               配置文件路径 [环境变量: RD_CONFIG=] [默认: config.yaml]
  -b, --bind <BIND>                   HTTP API 监听地址 [环境变量: RD_BIND=]
      --access-token <ACCESS_TOKEN>   API 访问令牌 [环境变量: RD_ACCESS_TOKEN=]
      --web-ui <WEB_UI>               Web 界面目录路径 [环境变量: RD_WEB_UI=]
      --write-config <WRITE_CONFIG>   将生成的配置写入指定路径
  -h, --help                          显示帮助信息
```

### 常用命令示例

1. 使用指定配置文件启动:

```bash
./rabbit-digger-pro -c my-config.yaml
```

2. 生成 JSON Schema:

```bash
./rabbit-digger-pro generate-schema > schema.json
```

3. 启动 HTTP API 服务:

```bash
./rabbit-digger-pro -b 127.0.0.1:8080 server
```

4. 设置访问令牌并启动:

```bash
./rabbit-digger-pro --access-token your-token server
```

### 环境变量

所有命令行参数都可以通过环境变量设置:

- `RD_CONFIG`: 配置文件路径
- `RD_BIND`: HTTP API 监听地址
- `RD_ACCESS_TOKEN`: API 访问令牌
- `RD_WEB_UI`: Web 界面目录路径

环境变量的优先级低于命令行参数。
