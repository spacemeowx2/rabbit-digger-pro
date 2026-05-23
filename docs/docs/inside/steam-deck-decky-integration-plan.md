---
sidebar: auto
---

# Steam Deck Decky 集成计划

## 目标

首要目标是降低 Steam Deck 上的安装、更新和迭代摩擦。开发期需要能快速把新版本装到 Deck 上测试；日常使用时，用户不应该频繁切到 Desktop Mode、打开浏览器，或者反复运行命令行。

Gaming Mode 的主操作面应该放在 Decky Loader 的右侧 Quick Access Menu 里。复杂设置可以放到 Decky 的大页面里，但不能把现有 Web UI 当成唯一入口。

## 产品形态

Steam Deck 上的 Rabbit Digger Pro 拆成三个表面：

```text
Gaming Mode
  Decky plugin
    - 代理开关
    - 当前状态
    - 节点选择
    - TUN 默认模式
    - Update 按钮
    - 最近错误和日志
    - 进入高级设置

Host
  rabbit-digger-pro helper
    - 作为 systemd user service 运行
    - 持有 Rust core 控制面和更新状态
    - 暴露本地 RPC / Web UI endpoint
    - 协调 TUN / global mode
    - 必要时调用固定 privileged helper

Desktop Mode
  可选 Web UI 或 Flatpak app
    - 导入订阅
    - 编辑复杂配置
    - 查看完整日志
    - 诊断和恢复
```

Decky plugin 是主用户体验。Web UI 仍然保留，但它不是 Steam Deck Gaming Mode 的主入口。

Steam Deck 是游戏主机，所以 Gaming Mode 的默认代理模式应该是 TUN/global routing。Local Proxy Mode 只作为 Desktop Mode、调试和权限不可用时的 fallback。

## 分发原则

命令行只能作为首次 bootstrap 入口。首次安装之后，正常更新必须能在 Decky 右侧菜单里点一下完成。

目标流程：

```text
首次安装:
  运行一次 bootstrap installer

日常使用:
  打开 Decky menu
  有更新时点击 Update
```

Bootstrap installer 负责安装第一个版本的：

- Rabbit Digger Pro helper binary
- systemd user service
- Decky plugin
- update metadata

Bootstrap 之后，Decky plugin 必须提供正常更新路径。用户不应该为了每次测试或升级反复跑同一条 `curl | sh` 命令。

## 为什么 Decky 优先

Flatpak 适合 Desktop Mode 应用，但不适合作为 Gaming Mode 的主交互模型。Gaming Mode 用户需要一个可以在游戏中打开的右侧菜单控制面板。

Decky Loader 已经提供这个表面。我们的 Decky plugin 应该是 host helper 的薄控制层，而不是第二套代理核心。

这样 ownership 也更清楚：

- Decky 负责 Gaming Mode 控制面。
- Host helper 负责进程生命周期、网络能力和持久状态。
- Web UI 负责复杂桌面配置。

## Decky 里的 Web UI

Decky 里做复杂设置有两条路：

1. 注册 Decky route，做一个 Decky-native 的大设置页。
2. 用浏览器或 iframe-like surface 嵌入现有 Web UI。

主路径选第 1 种。

现有 Web UI 不是按 Steam Deck 的焦点控制、手柄导航、右侧菜单约束和 Steam UI 视觉规范设计的。嵌入 Web UI 可以作为 fallback，但不应该是主界面。Decky-native 页面可以用 Steam 风格控件、原生焦点行为、tabs 和 route navigation。

推荐布局：

```text
QAM panel
  Status
  Connect / Disconnect
  Selected node
  Update
  Recent error
  Error notification
  Advanced Settings

Decky advanced route
  Overview
  Nodes
  Subscriptions
  Logs
  Updates
  TUN / Game Mode
  Diagnostics

Web UI
  Open in browser / Desktop Mode fallback
```

## Helper 架构

Decky plugin 不应该在 plugin 进程里直接跑 Rust core。它应该和 host 上的 systemd user service helper 通信。

User-level helper 路径：

```text
binary:
  ~/.local/share/rabbit_digger_pro/helper/rabbit-digger-pro

systemd unit:
  ~/.config/systemd/user/rabbit-digger-pro.service
```

Unit 运行：

```text
rabbit-digger-pro service run --bind 127.0.0.1:9091 --access-token <token>
```

Helper 需要通过 RPC 暴露版本和健康状态：

```text
deck.info
deck.helper.status
deck.helper.start
deck.helper.stop
deck.helper.restart
deck.helper.version
deck.update.check
deck.update.apply
deck.tun.status
deck.tun.enable
deck.tun.disable
```

Decky Python backend 只做窄适配层。它可以读本地 token/config、调用本地 RPC、执行固定 service/update 命令，但不能暴露任意命令执行。

User service 是控制面和 updater owner。TUN/default Game Mode 涉及 route、rule、DNS、kill-switch 和 `/dev/net/tun`，需要固定 privileged helper 或受控提权流程来执行。Decky plugin 和普通 user service 都不应该直接拿任意 shell/root 权限。

## 更新流程

右侧菜单里的 updater 应该基于带 checksum 的 manifest。后续如果需要可以再加签名。

Manifest 示例：

```json
{
  "version": "0.1.12",
  "channel": "dev",
  "published_at": "2026-05-18T00:00:00Z",
  "helper": {
    "url": "https://github.com/spacemeowx2/rabbit-digger-pro/releases/download/v0.1.12/rabbit-digger-pro-x86_64-unknown-linux-gnu",
    "sha256": "..."
  },
  "decky_plugin": {
    "url": "https://github.com/spacemeowx2/rabbit-digger-pro/releases/download/v0.1.12/rabbit-digger-pro-decky.zip",
    "sha256": "..."
  }
}
```

更新步骤：

1. 获取 update manifest。
2. 比较 plugin/helper/core 版本。
3. 下载 assets 到 staging directory。
4. 安装前校验 SHA-256。
5. 所有 assets 校验通过后再停止 helper。
6. 把 helper 安装到 versioned directory。
7. 原子切换 `current` 到新版本。
8. 重启 systemd user service。
9. Staging plugin replacement。
10. 必要时 reload plugin 或 plugin loader。

Updater 需要保留上一个 helper 版本，方便失败时 rollback。

Plugin self-update 比 helper update 风险更高，因为 plugin 正在替换当前加载的代码。把它当成 staged operation：

```text
download -> verify -> stage -> spawn updater -> replace plugin directory -> reload Decky/plugin loader
```

如果 plugin self-reload 不可靠，UI 必须明确提示用户 reload Decky 或重启 Gaming Mode。Helper 更新仍然应该保持一键完成。

## Channels

使用两个 release channel：

```text
stable
  慢一些，面向普通用户

dev
  快速迭代，面向本地 Steam Deck 测试
```

Decky UI 需要允许用户选择 channel。早期手动安装版本默认 `dev`；上架后 store build 默认 `stable`。

## TUN / Game Mode

Local Proxy Mode 对 Desktop Mode、调试和部分 app 仍然有用，但 Steam Deck 的主要场景是玩游戏，Gaming Mode 默认应该以 TUN/global routing 为目标。这个能力不应该被产品命名成 Enhanced Mode；它是 Game Mode 下的默认代理模式。

实现上仍然要把权限、DNS 和 rollback 做严谨。也就是说：TUN 是产品默认路径，但不是静默启用的危险操作。首次启用时需要明确授权和清晰状态说明；授权完成后，Decky 的主开关默认控制 TUN。

TUN 实现应该分阶段验证。第一阶段先不接远端代理，而是把 Steam Deck 上所有 UDP 流量通过 TUN 收到 `rabbit-digger-pro` 进程里，再由进程按 direct path 发出去。这个阶段的目标是证明路由、TUN 收包、UDP 转发、rollback 和 Decky 状态展示都成立。第二阶段再把这条 UDP path 接到选中的代理节点。

要求：

- Gaming Mode 默认目标是 TUN/global routing。
- 首次启用需要用户明确授权。
- 使用固定 root/system helper 路径或受控提权流程。
- 默认不写 `/etc/resolv.conf`。
- 对 route、rule、DNS、kill-switch side effects 有清晰 rollback。
- 失败细节能在 Decky UI 里显示。
- 代理不可用时，Decky QAM 必须弹出明确提示，并在状态区保留可查看的错误原因。

DNS setup 应该策略化：

```text
none
resolvectl
network-manager
resolv-conf
```

Steam Deck 默认应该是 `none` 或 `resolvectl`，而不是直接写 `/etc/resolv.conf`。

## 安全规则

- 所有 helper RPC 都要求 access token。
- Token 不打印到日志。
- Decky backend 不能运行任意 shell command。
- Update 下载后必须先校验再安装。
- Helper install paths 固定。
- Service unit 内容由代码生成，不接受用户输入的 unit text。
- TUN/root 操作和 Local Proxy fallback 隔离。
- 更新失败时必须保留上一个可运行 helper。

## 实施计划

实施计划只拆两个交付 PR。目标不是把内部模块切细，而是尽快得到一个可以装、可以更新、可以在 Steam Deck 上反复测试的发布面。

### PR 1: Release, Install, And Update

第一个 PR 就交付可发布版本。它需要把 helper、Decky plugin、bootstrap installer、release assets 和 Decky 内一键更新放在同一个交付里。

范围：

- 发布 Steam Deck Linux helper binary。
- 发布 Decky plugin zip。
- 新增 `install-steamdeck.sh`，用于首次 bootstrap。
- 新增 update manifest，记录 helper/plugin 版本、下载地址和 SHA-256。
- 安装 helper 到固定 user path。
- 安装并启用 systemd user service。
- 安装 Decky plugin。
- Decky QAM panel 显示 helper 状态、版本、更新状态、最近错误。
- Decky QAM panel 提供 Update 按钮。
- Update 按钮能下载、校验、安装 helper/plugin，并重启 helper。
- 更新失败时保留上一个可运行 helper。

验收：

- 干净 Steam Deck 可以通过一条命令完成首次安装。
- 首次安装后，用户不需要再反复跑命令更新。
- Decky 右侧菜单能看到当前版本和可用更新。
- 点击 Update 可以完成 helper/core 更新。
- Plugin 自更新如果不能可靠热重载，UI 必须明确提示 reload Decky 或重启 Gaming Mode。
- 更新失败时 Decky 显示错误，旧版本仍可运行。

PR 1 不要求 TUN/UDP 已经工作。它的目标是先把发布、安装、更新闭环打通，因为这是快速迭代的前提。

### PR 2: TUN UDP OK

第二个 PR 交付 Steam Deck 游戏场景的 UDP 网络能力。它可以在实现上分阶段做 direct baseline 和 proxy path，但不再拆成多个 PR。

范围：

- 首次启用 TUN 时走明确授权或固定 privileged helper。
- 配置 route/rule，让所有 UDP 流量进入 `rabbit-digger-pro` 进程。
- 进程内先提供 direct UDP egress，证明 TUN 收包和转发成立。
- Decky 显示 captured/forwarded UDP counters。
- 在 direct baseline 成立后，把 UDP egress 接到选中的代理节点。
- 切换到代理前做可用性检查。
- 代理不可用时，Decky QAM 弹出明确提示。
- 错误状态保留节点名、失败原因和发生时间。
- 默认不静默 fallback 到 direct，除非用户明确启用 direct fallback。
- Stop/crash recovery 清理 route/rule side effects。
- DNS setup mode 保持显式和保守，默认不直接写 `/etc/resolv.conf`。

验收：

- Decky 主开关默认控制 TUN。
- 所有 UDP 流量能被 TUN 收到 `rabbit-digger-pro` 进程里。
- 进程能用 direct path 把 UDP 发出去。
- UDP 流量可以切到选中的代理节点。
- 代理不可用时，Decky 右侧菜单弹出提示。
- Decky UI 能显示 captured/forwarded UDP 状态和最近错误。
- Stop/crash recovery 能清理系统网络改动。

## 第一个里程碑

第一个里程碑就是 PR 1：发布、首次安装和 Decky 内更新闭环。

```text
create release assets
bootstrap once
open Gaming Mode
open Decky panel
see helper/plugin version
click Update
download and verify update assets
install update
restart helper
show errors if update fails
keep previous helper runnable
```

UDP/TUN 不应该阻塞第一个里程碑。先把安装和更新摩擦降下来，后面才能在 Steam Deck 上快速测 UDP。

## 参考

- Decky Loader: https://github.com/SteamDeckHomebrew/decky-loader
- Decky plugin template: https://github.com/SteamDeckHomebrew/decky-plugin-template
- Decky plugin database: https://github.com/SteamDeckHomebrew/decky-plugin-database
- Decky AutoFlatpaks: https://github.com/jurassicplayer/decky-autoflatpaks
- Xray Decky reference: https://github.com/VadimOnix/xray-decky
