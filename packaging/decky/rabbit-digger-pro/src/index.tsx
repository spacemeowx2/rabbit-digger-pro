import {
  ButtonItem,
  PanelSection,
  PanelSectionRow,
  Spinner,
  staticClasses,
  ToggleField,
} from "@decky/ui";
import { definePlugin, toaster } from "@decky/api";
import { useEffect, useRef, useState } from "react";

type ProtectionState = "on" | "attention" | "starting" | "off";

type HelperStatus = {
  installed: boolean;
  install_mode: "daemon" | "system" | "user" | "missing";
  active: boolean;
  engine_active: boolean;
  system_active: boolean;
  system_enabled: boolean;
  user_active: boolean;
  user_enabled: boolean;
  tun_active: boolean;
  tun_name: string;
  tun_addresses: string[];
  dns_active: boolean;
  dns_servers: string[];
  protection: ProtectionState;
  summary: string;
  helper_path: string;
  helper_version: string | null;
  plugin_version: string;
  update_available: boolean;
  latest_version: string | null;
  last_error: string | null;
};

type RuntimeStats = {
  timestamp_ms: number;
  active_connections: number;
  tcp_connections: number;
  udp_connections: number;
  total_upload: number;
  total_download: number;
  rx_bytes: number;
  tx_bytes: number;
  rx_packets: number;
  tx_packets: number;
};

type UpdateResult = {
  ok: boolean;
  version: string | null;
  needs_reload: boolean;
  error: string | null;
};

type NodeInfo = {
  name: string;
  selected: boolean;
  type: string | null;
};

type NodeDelayStatus = "ok" | "timeout" | "error" | "testing";

type NodeDelayResult = {
  name: string;
  status: NodeDelayStatus;
  connect: number | null;
  response: number | null;
  latency: number | null;
  error: string | null;
  tested_at: number;
};

type NodesResult = {
  net_name: string;
  selected: string | null;
  delay_url: string;
  nodes: NodeInfo[];
  results: Record<string, NodeDelayResult>;
  error?: string;
};

type TrafficPoint = {
  id: number;
  downRate: number;
  upRate: number;
};

type LogsResult = {
  logs: string;
};

type EngineStatusName = "Idle" | "Starting" | "Running" | "Stopping" | "Error" | "Connecting";

type EngineStatus = {
  status: EngineStatusName;
  message?: string;
};

type ServerEvent = {
  event: string;
  status?: EngineStatus;
};

type ConnectionEntry = {
  protocol?: string;
  upload?: number;
  download?: number;
};

type ConnectionSnapshot = {
  connections: Record<string, ConnectionEntry>;
  total_upload: number;
  total_download: number;
};

type NetConfig = {
  type: string;
  selected?: string;
  list?: string[];
};

type RdpConfig = {
  net?: Record<string, NetConfig>;
};

type UserdataItem = {
  content: string;
};

type RpcErrorPayload = {
  code: number;
  message: string;
  data?: unknown;
};

type RpcLogEntry = Record<string, any>;

type SubscriptionHandler = (payload: unknown) => void;

type SubscriptionRecord = {
  key: string;
  topic: string;
  params: unknown;
  handlers: Set<SubscriptionHandler>;
  serverId: string | null;
  subscribePromise: Promise<void> | null;
};

const DAEMON_RPC_URL = "ws://127.0.0.1:9091/api/rpc";
const LAST_SOURCE_KEY = "daemon/last_source";
const SELECTOR_NET = "rdp_selected";
const NODE_DELAY_URL = "http://store.steampowered.com/";
const NODE_DELAY_TIMEOUT_MS = 5000;
const EMPTY_CONNECTIONS: ConnectionSnapshot = {
  connections: {},
  total_upload: 0,
  total_download: 0,
};

class RpcClient {
  private socket: WebSocket | null = null;
  private connectPromise: Promise<void> | null = null;
  private reconnectTimer: number | null = null;
  private nextId = 1;
  private pending = new Map<number, { resolve: (value: unknown) => void; reject: (error: Error) => void }>();
  private subscriptions = new Map<string, SubscriptionRecord>();

  async request<T>(method: string, params: unknown = {}): Promise<T> {
    await this.ensureConnected();
    const id = this.nextId++;
    return await new Promise<T>((resolve, reject) => {
      this.pending.set(id, { resolve: resolve as (value: unknown) => void, reject });
      try {
        this.socket?.send(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
      } catch (error) {
        this.pending.delete(id);
        reject(error instanceof Error ? error : new Error(String(error)));
      }
    });
  }

  subscribe(topic: string, params: unknown, handler: SubscriptionHandler): () => void {
    const key = JSON.stringify({ topic, params });
    let record = this.subscriptions.get(key);
    if (!record) {
      record = { key, topic, params, handlers: new Set(), serverId: null, subscribePromise: null };
      this.subscriptions.set(key, record);
    }
    record.handlers.add(handler);
    void this.ensureSubscription(record);

    return () => {
      const current = this.subscriptions.get(key);
      if (!current) {
        return;
      }
      current.handlers.delete(handler);
      if (current.handlers.size === 0) {
        this.subscriptions.delete(key);
        const serverId = current.serverId;
        current.serverId = null;
        if (serverId) {
          void this.request("rpc.unsubscribe", { subscription: serverId }).catch(() => undefined);
        }
      }
    };
  }

  private async ensureConnected(): Promise<void> {
    if (this.socket?.readyState === WebSocket.OPEN) {
      return;
    }
    if (this.connectPromise) {
      return await this.connectPromise;
    }

    this.connectPromise = new Promise<void>((resolve, reject) => {
      const socket = new WebSocket(DAEMON_RPC_URL);
      this.socket = socket;

      socket.onopen = () => {
        this.connectPromise = null;
        for (const subscription of this.subscriptions.values()) {
          subscription.serverId = null;
          subscription.subscribePromise = null;
          void this.ensureSubscription(subscription);
        }
        resolve();
      };

      socket.onmessage = (event) => this.handleMessage(String(event.data));
      socket.onerror = () => {
        if (socket.readyState !== WebSocket.OPEN) {
          this.connectPromise = null;
          reject(new Error("Rabbit Digger Pro daemon is not reachable"));
        }
      };
      socket.onclose = () => {
        if (this.socket === socket) {
          this.socket = null;
        }
        if (this.connectPromise) {
          this.connectPromise = null;
          reject(new Error("Rabbit Digger Pro daemon connection closed"));
        }
        for (const [id, pending] of this.pending) {
          pending.reject(new Error("Rabbit Digger Pro daemon connection closed"));
          this.pending.delete(id);
        }
        this.scheduleReconnect();
      };
    });

    return await this.connectPromise;
  }

  private scheduleReconnect() {
    if (this.reconnectTimer !== null) {
      return;
    }
    this.reconnectTimer = window.setTimeout(() => {
      this.reconnectTimer = null;
      void this.ensureConnected().catch(() => undefined);
    }, 1500);
  }

  private async ensureSubscription(record: SubscriptionRecord): Promise<void> {
    if (record.serverId || record.subscribePromise || record.handlers.size === 0) {
      return;
    }
    record.subscribePromise = (async () => {
      try {
        const result = await this.request<{ subscription: string }>("rpc.subscribe", {
          topic: record.topic,
          params: record.params,
        });
        if (!this.subscriptions.has(record.key)) {
          await this.request("rpc.unsubscribe", { subscription: result.subscription }).catch(() => undefined);
          return;
        }
        record.serverId = result.subscription;
      } finally {
        record.subscribePromise = null;
      }
    })();
    await record.subscribePromise;
  }

  private handleMessage(raw: string) {
    const message = JSON.parse(raw);
    if (message.method === "rpc.subscription") {
      const serverId = message.params?.subscription;
      for (const subscription of this.subscriptions.values()) {
        if (subscription.serverId === serverId) {
          for (const handler of subscription.handlers) {
            handler(message.params?.payload);
          }
          break;
        }
      }
      return;
    }

    if (typeof message.id === "number") {
      const pending = this.pending.get(message.id);
      if (!pending) {
        return;
      }
      this.pending.delete(message.id);
      if (message.error) {
        const error = message.error as RpcErrorPayload;
        pending.reject(new Error(error.data ? `${error.message}: ${JSON.stringify(error.data)}` : error.message));
      } else {
        pending.resolve(message.result);
      }
    }
  }
}

const rpcClient = new RpcClient();

function rpc<T>(method: string, params: unknown = {}): Promise<T> {
  return rpcClient.request<T>(method, params);
}

function subscribe(topic: string, params: unknown, handler: SubscriptionHandler) {
  return rpcClient.subscribe(topic, params, handler);
}

function protectionFromEngine(engineStatus: EngineStatus, connected: boolean): ProtectionState {
  if (!connected) {
    return "off";
  }
  if (engineStatus.status === "Running") {
    return "on";
  }
  if (engineStatus.status === "Starting" || engineStatus.status === "Stopping") {
    return "starting";
  }
  if (engineStatus.status === "Error") {
    return "attention";
  }
  return "off";
}

function statusFromEngine(engineStatus: EngineStatus, connected: boolean): HelperStatus {
  const protection = protectionFromEngine(engineStatus, connected);
  return {
    installed: connected,
    install_mode: connected ? "daemon" : "missing",
    active: connected,
    engine_active: engineStatus.status === "Running" || engineStatus.status === "Starting",
    system_active: connected,
    system_enabled: true,
    user_active: false,
    user_enabled: false,
    tun_active: false,
    tun_name: "",
    tun_addresses: [],
    dns_active: false,
    dns_servers: [],
    protection,
    summary: engineStatus.status,
    helper_path: "127.0.0.1:9091",
    helper_version: null,
    plugin_version: "0.1.0",
    update_available: false,
    latest_version: null,
    last_error: connected ? engineStatus.message ?? null : "Rabbit Digger Pro daemon is not reachable",
  };
}

function nodesFromConfig(config: RdpConfig | null, previous?: NodesResult | null): NodesResult {
  const nets = config?.net ?? {};
  const selectorName = nets[SELECTOR_NET]?.type === "select"
    ? SELECTOR_NET
    : Object.entries(nets).find(([, net]) => net.type === "select")?.[0];
  if (!selectorName) {
    return {
      net_name: SELECTOR_NET,
      selected: null,
      delay_url: NODE_DELAY_URL,
      nodes: [],
      results: previous?.results ?? {},
      error: "No selectable proxy nodes found",
    };
  }

  const selector = nets[selectorName];
  const selected = typeof selector.selected === "string" ? selector.selected : null;
  const options = Array.isArray(selector.list) ? selector.list : [];
  return {
    net_name: selectorName,
    selected,
    delay_url: NODE_DELAY_URL,
    nodes: options.map((name) => ({
      name,
      selected: name === selected,
      type: nets[name]?.type ?? null,
    })),
    results: previous?.results ?? {},
  };
}

async function loadNodes(previous?: NodesResult | null): Promise<NodesResult> {
  const config = await rpc<RdpConfig | null>("config.get");
  return nodesFromConfig(config, previous);
}

async function applyLastConfig() {
  const item = await rpc<UserdataItem>("userdata.get", { path: LAST_SOURCE_KEY });
  const source = JSON.parse(item.content) as Record<string, unknown>;
  await rpc<null>("config.apply", source);
}

async function getLogs(): Promise<LogsResult> {
  const entries = await rpc<RpcLogEntry[]>("logs.tail", { tail: 120 });
  return { logs: entries.map(formatLogEntry).join("\n") };
}

function formatLogEntry(entry: RpcLogEntry) {
  const message = entry.fields?.message ?? entry.message;
  return [entry.level, entry.target, message].filter(Boolean).join(" ") || JSON.stringify(entry);
}

type Locale = "en" | "zh";

const copy = {
  en: {
    sectionGameProxy: "Game Proxy",
    sectionNodes: "Nodes",
    sectionUpdates: "Updates",
    sectionDetails: "Details",
    loading: "Loading...",
    dns: "DNS",
    routed: "Routed",
    normalNetwork: "Normal network",
    on: "On",
    off: "Off",
    attached: "Attached",
    notAttached: "Not attached",
    notChecked: "Not checked",
    upToDate: "Up to date",
    updateTo: (version: string) => `Update to ${version}`,
    gameProxy: "Game proxy",
    gameProxyDesc: "Route game and system traffic through Rabbit Digger Pro.",
    startOk: "Game proxy is on",
    stopOk: "Game proxy is off",
    restartLabel: "Connection",
    restartDesc: "Re-apply the current node and tunnel route.",
    restartButton: "Restart",
    restartOk: "Game proxy restarted",
    currentNode: "Current node",
    noNodeSelected: "No node selected",
    nodeCount: (count: number) => `${count} nodes`,
    probeNodes: "Test nodes",
    autoSelect: "Fastest",
    probingNodes: "Testing nodes...",
    switchingNode: "Switching node...",
    useNode: "Use",
    selectedNode: "Current",
    nodeProbeDesc: "Measure latency through each proxy node.",
    autoSelectDesc: "Test nodes and switch to the fastest reachable one.",
    nodeSwitchOk: (name: string) => `Using ${name}`,
    fastestSelected: (name: string, latency: string) => `${name} selected (${latency})`,
    noReachableNode: "No reachable node found",
    noNodes: "No proxy nodes found",
    nodeLoadFailed: "Could not load nodes",
    nodeDelayTesting: "Testing",
    nodeDelayTimeout: "Timeout",
    nodeDelayError: "Unavailable",
    nodeDelayIdle: "Not tested",
    runtimeTitle: "Live traffic",
    activeConnections: "Connections",
    tcpUdp: "TCP / UDP",
    speed: "Speed",
    packets: "Packets",
    totalTraffic: "Traffic",
    noRuntimeStats: "Waiting for traffic stats",
    updateLabel: "Version",
    updateDesc: "Check and install the latest Deck build.",
    updateButton: "Update",
    updateFailed: "Update failed",
    updatedTitle: "Rabbit Digger Pro updated",
    updatedNeedsReload: "Restart Decky Loader or reopen Gaming Mode to finish the menu update",
    installedVersion: (version: string) => `Installed ${version}`,
    latestVersion: "latest version",
    unknownError: "Unknown error",
    logsFailed: "Could not load logs",
    noLogs: "No logs yet",
    controlMode: "Control mode",
    gameModeService: "Game Mode service",
    protection: "Protection",
    backend: "Backend",
    tunnel: "Tunnel",
    address: "Address",
    nameServer: "Name server",
    plugin: "Plugin",
    running: "Running",
    stopped: "Stopped",
    none: "None",
    statusLabel: "Status",
    statusDesc: "Refresh service and tunnel state.",
    refresh: "Refresh",
    logsLabel: "Logs",
    logsDesc: "Show recent plugin and daemon logs.",
    show: "Show",
    hide: "Hide",
    notTested: "Not tested",
    status: {
      on: {
        title: "Game proxy is on",
        tone: "Games are using Rabbit Digger Pro.",
        color: "#7ee787",
        background: "rgba(126, 231, 135, 0.12)",
      },
      attention: {
        title: "Needs attention",
        tone: "The tunnel is running, but DNS needs a check.",
        color: "#ffd166",
        background: "rgba(255, 209, 102, 0.13)",
      },
      starting: {
        title: "Starting",
        tone: "The service is running; the tunnel is not ready yet.",
        color: "#9ecbff",
        background: "rgba(158, 203, 255, 0.12)",
      },
      off: {
        title: "Game proxy is off",
        tone: "Games are using the normal network.",
        color: "#ff8a8a",
        background: "rgba(255, 138, 138, 0.12)",
      },
    },
  },
  zh: {
    sectionGameProxy: "游戏代理",
    sectionNodes: "节点",
    sectionUpdates: "更新",
    sectionDetails: "详情",
    loading: "正在加载...",
    dns: "DNS",
    routed: "已接管",
    normalNetwork: "普通网络",
    on: "开",
    off: "关",
    attached: "已接管",
    notAttached: "未接管",
    notChecked: "未检查",
    upToDate: "已是最新",
    updateTo: (version: string) => `可更新到 ${version}`,
    gameProxy: "游戏代理",
    gameProxyDesc: "接管游戏和系统网络流量。",
    startOk: "游戏代理已开启",
    stopOk: "游戏代理已关闭",
    restartLabel: "连接",
    restartDesc: "重新应用当前节点和隧道路由。",
    restartButton: "重启",
    restartOk: "游戏代理已重启",
    currentNode: "当前节点",
    noNodeSelected: "未选择节点",
    nodeCount: (count: number) => `${count} 个节点`,
    probeNodes: "测速",
    autoSelect: "最快节点",
    probingNodes: "正在测速...",
    switchingNode: "正在切换...",
    useNode: "使用",
    selectedNode: "当前",
    nodeProbeDesc: "测试每个代理节点到 Steam 的实际延迟。",
    autoSelectDesc: "测速后切到当前最快可用节点。",
    nodeSwitchOk: (name: string) => `已切换到 ${name}`,
    fastestSelected: (name: string, latency: string) => `已选择 ${name}（${latency}）`,
    noReachableNode: "没有可用节点",
    noNodes: "没有找到可切换节点",
    nodeLoadFailed: "无法读取节点",
    nodeDelayTesting: "测速中",
    nodeDelayTimeout: "超时",
    nodeDelayError: "不可用",
    nodeDelayIdle: "未测速",
    runtimeTitle: "实时流量",
    activeConnections: "连接数",
    tcpUdp: "TCP / UDP",
    speed: "速率",
    packets: "数据包",
    totalTraffic: "累计流量",
    noRuntimeStats: "等待流量数据",
    updateLabel: "版本",
    updateDesc: "检查并安装最新的 Steam Deck 版本。",
    updateButton: "更新",
    updateFailed: "更新失败",
    updatedTitle: "Rabbit Digger Pro 已更新",
    updatedNeedsReload: "重启 Decky Loader 或重新进入游戏模式以完成菜单更新",
    installedVersion: (version: string) => `已安装 ${version}`,
    latestVersion: "最新版本",
    unknownError: "未知错误",
    logsFailed: "无法读取日志",
    noLogs: "暂无日志",
    controlMode: "控制模式",
    gameModeService: "游戏模式服务",
    protection: "代理状态",
    backend: "后台",
    tunnel: "隧道",
    address: "地址",
    nameServer: "DNS 服务器",
    plugin: "插件",
    running: "运行中",
    stopped: "已停止",
    none: "无",
    statusLabel: "状态",
    statusDesc: "刷新服务和隧道状态。",
    refresh: "刷新",
    logsLabel: "日志",
    logsDesc: "查看最近的插件和 daemon 日志。",
    show: "查看",
    hide: "隐藏",
    notTested: "未测试",
    status: {
      on: {
        title: "游戏代理已开启",
        tone: "游戏流量正在通过 Rabbit Digger Pro。",
        color: "#7ee787",
        background: "rgba(126, 231, 135, 0.12)",
      },
      attention: {
        title: "需要处理",
        tone: "隧道已运行，但 DNS 需要检查。",
        color: "#ffd166",
        background: "rgba(255, 209, 102, 0.13)",
      },
      starting: {
        title: "正在启动",
        tone: "后台服务已运行，隧道还没准备好。",
        color: "#9ecbff",
        background: "rgba(158, 203, 255, 0.12)",
      },
      off: {
        title: "游戏代理已关闭",
        tone: "游戏正在使用普通网络。",
        color: "#ff8a8a",
        background: "rgba(255, 138, 138, 0.12)",
      },
    },
  },
};

function locale(): Locale {
  const languages = [navigator.language, ...(navigator.languages ?? [])]
    .filter(Boolean)
    .map((language) => language.toLowerCase());
  return languages.some((language) => language.startsWith("zh")) ? "zh" : "en";
}

function DetailRow({ label, value }: { label: string; value: string }) {
  return (
    <div style={{ display: "flex", justifyContent: "space-between", gap: "12px" }}>
      <span style={{ color: "#8f98a0" }}>{label}</span>
      <span style={{ maxWidth: "58%", textAlign: "right" }}>{value}</span>
    </div>
  );
}

function formatBytes(value: number) {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let next = Math.max(0, value);
  let unit = 0;
  while (next >= 1024 && unit < units.length - 1) {
    next /= 1024;
    unit += 1;
  }
  const digits = unit === 0 || next >= 10 ? 0 : 1;
  return `${next.toFixed(digits)} ${units[unit]}`;
}

function formatRate(value: number) {
  return `${formatBytes(value)}/s`;
}

function userFacingMessage(message: string, currentLocale: Locale) {
  const t = copy[currentLocale];
  if (/Engine not running/i.test(message)) {
    return currentLocale === "zh" ? "游戏代理未运行。" : "Game proxy is not running.";
  }
  if (/ConnectionReset|Connection reset/i.test(message)) {
    return currentLocale === "zh"
      ? "当前节点断开连接，请换节点重试。"
      : "Selected node closed the connection. Try another node.";
  }
  if (/reality|handshake|TLS Error|certificate|UnexpectedEof|eof/i.test(message)) {
    return currentLocale === "zh"
      ? "当前节点握手失败，请换节点重试。"
      : "Selected node handshake failed. Try another node.";
  }
  if (/NetworkUnreachable|Network is unreachable/i.test(message)) {
    return currentLocale === "zh" ? "网络路由不可用。" : "Network route is unavailable.";
  }
  if (/timeout|Elapsed/i.test(message)) {
    return currentLocale === "zh" ? "当前节点连接超时。" : "Selected node timed out.";
  }
  if (/Not tested/i.test(message)) {
    return t.notTested;
  }
  if (message.length > 72) {
    return `${message.slice(0, 69)}...`;
  }
  return message;
}

function Sparkline({ points }: { points: TrafficPoint[] }) {
  const width = 220;
  const height = 44;
  const values = points.map((point) => point.downRate + point.upRate);
  const max = Math.max(1, ...values);
  const path = values
    .map((value, index) => {
      const x = values.length <= 1 ? 0 : (index / (values.length - 1)) * width;
      const y = height - (value / max) * (height - 4) - 2;
      return `${index === 0 ? "M" : "L"} ${x.toFixed(1)} ${y.toFixed(1)}`;
    })
    .join(" ");

  return (
    <svg
      viewBox={`0 0 ${width} ${height}`}
      width="100%"
      height="44"
      preserveAspectRatio="none"
      aria-hidden="true"
      style={{ display: "block" }}
    >
      <path d={`M 0 ${height - 1} H ${width}`} stroke="rgba(255,255,255,0.12)" />
      <path d={path} fill="none" stroke="#66d9ef" strokeWidth="2.5" />
    </svg>
  );
}

function ControlIcon({
  name,
}: {
  name: "proxy" | "restart" | "update" | "test" | "status" | "logs" | "nodes";
}) {
  const paths = {
    proxy: "M4 12h5m6 0h5M9 12a3 3 0 1 0 6 0 3 3 0 0 0-6 0ZM7 6l2 2m6 8 2 2M17 6l-2 2m-6 8-2 2",
    restart: "M7 7h8a4 4 0 0 1 0 8h-4m-4-8 3-3m-3 3 3 3",
    update: "M12 4v10m0 0 4-4m-4 4-4-4M5 18h14",
    test: "M13 3 5 14h6l-1 7 8-11h-6l1-7Z",
    status: "M5 12l4 4L19 6",
    logs: "M7 5h10M7 9h10M7 13h7M5 19h14V3H5v16Z",
    nodes: "M4 7h7m2 0h7M4 17h7m2 0h7M8 7a2 2 0 1 0 4 0 2 2 0 0 0-4 0Zm4 10a2 2 0 1 0 4 0 2 2 0 0 0-4 0Z",
  };

  return (
    <svg
      viewBox="0 0 24 24"
      width="22"
      height="22"
      fill="none"
      aria-hidden="true"
      style={{ display: "block" }}
    >
      <path
        d={paths[name]}
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function RabbitDiggerIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      width="24"
      height="24"
      fill="none"
      aria-hidden="true"
      style={{ display: "block" }}
    >
      <path
        d="M7.5 9.5h9M7.5 14.5h9M9 5.75l-4.5 6.25L9 18.25M15 5.75l4.5 6.25L15 18.25"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <circle cx="12" cy="12" r="2" fill="currentColor" />
    </svg>
  );
}

function RuntimePanel({
  status,
  stats,
  history,
  currentLocale,
}: {
  status: HelperStatus | null;
  stats: RuntimeStats | null;
  history: TrafficPoint[];
  currentLocale: Locale;
}) {
  const t = copy[currentLocale];
  if (!status) {
    return <div style={{ minHeight: "74px" }}>{t.loading}</div>;
  }

  const state = t.status[status.protection] ?? t.status.off;
  const showIssue = status.protection === "attention" || status.protection === "starting" || Boolean(status.last_error);
  const latest = history[history.length - 1] ?? { downRate: 0, upRate: 0 };
  const packetCount = (stats?.rx_packets ?? 0) + (stats?.tx_packets ?? 0);
  const totalTraffic = (stats?.rx_bytes ?? 0) + (stats?.tx_bytes ?? 0);

  return (
    <div style={{ display: "grid", gap: "10px", minHeight: "122px" }}>
      {showIssue ? (
        <div
          style={{
            padding: "12px",
            borderRadius: "6px",
            background: state.background,
            border: `1px solid ${state.color}`,
          }}
        >
          <div style={{ color: state.color, fontSize: "16px", fontWeight: 700 }}>
            {state.title}
          </div>
          <div style={{ marginTop: "4px", color: "#d8dee9", fontSize: "13px" }}>
            {status.last_error ? userFacingMessage(status.last_error, currentLocale) : state.tone}
          </div>
        </div>
      ) : null}

      <div style={{ display: "flex", justifyContent: "space-between", gap: "12px" }}>
        <div>
          <div style={{ color: "#8f98a0", fontSize: "12px", fontWeight: 700 }}>
            {t.runtimeTitle}
          </div>
          <div style={{ color: "#f1f3f5", fontSize: "20px", fontWeight: 800 }}>
            ↓ {formatRate(latest.downRate)}
          </div>
        </div>
        <div style={{ color: "#d8dee9", fontSize: "13px", textAlign: "right" }}>
          <div>↑ {formatRate(latest.upRate)}</div>
          <div style={{ color: "#8f98a0" }}>{stats ? formatBytes(totalTraffic) : t.noRuntimeStats}</div>
        </div>
      </div>

      <Sparkline points={history.length ? history : [{ id: 0, downRate: 0, upRate: 0 }]} />

      <div
        style={{
          display: "grid",
          gridTemplateColumns: "repeat(3, 1fr)",
          gap: "8px",
          fontSize: "12px",
        }}
      >
        <div>
          <div style={{ color: "#8f98a0" }}>{t.activeConnections}</div>
          <div style={{ fontWeight: 700 }}>{stats?.active_connections ?? 0}</div>
        </div>
        <div>
          <div style={{ color: "#8f98a0" }}>{t.tcpUdp}</div>
          <div style={{ fontWeight: 700 }}>
            {stats?.tcp_connections ?? 0} / {stats?.udp_connections ?? 0}
          </div>
        </div>
        <div>
          <div style={{ color: "#8f98a0" }}>{t.packets}</div>
          <div style={{ fontWeight: 700 }}>{packetCount.toLocaleString()}</div>
        </div>
      </div>
    </div>
  );
}

function formatLatency(value: number | null | undefined) {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return null;
  }
  return `${Math.max(0, Math.round(value))}ms`;
}

function nodeDelayLabel(
  result: NodeDelayResult | undefined,
  currentLocale: Locale,
): string {
  const t = copy[currentLocale];
  if (!result) {
    return t.nodeDelayIdle;
  }
  if (result.status === "testing") {
    return t.nodeDelayTesting;
  }
  if (result.status === "timeout") {
    return t.nodeDelayTimeout;
  }
  if (result.status === "error") {
    return t.nodeDelayError;
  }
  return formatLatency(result.latency ?? result.response ?? result.connect) ?? t.nodeDelayIdle;
}

function nodeDelayColor(result: NodeDelayResult | undefined) {
  if (!result || result.status === "testing") {
    return "#8f98a0";
  }
  if (result.status === "timeout" || result.status === "error") {
    return "#ff8a8a";
  }
  const latency = result.latency ?? result.response ?? result.connect ?? 9999;
  if (latency <= 220) {
    return "#7ee787";
  }
  if (latency <= 650) {
    return "#ffd166";
  }
  return "#ffb86c";
}

function findFastestNode(nodes: NodesResult | null) {
  if (!nodes) {
    return null;
  }
  return Object.values(nodes.results)
    .filter((result) => result.status === "ok" && typeof result.latency === "number")
    .sort((left, right) => (left.latency ?? 999999) - (right.latency ?? 999999))[0] ?? null;
}

function NodeStatusDot({ result, selected }: { result?: NodeDelayResult; selected: boolean }) {
  const color = selected ? "#66d9ef" : nodeDelayColor(result);
  return (
    <span
      style={{
        display: "inline-block",
        width: "10px",
        height: "10px",
        borderRadius: "999px",
        background: color,
        boxShadow: selected ? `0 0 0 3px rgba(102, 217, 239, 0.18)` : "none",
      }}
    />
  );
}

function NodesPanel({
  nodes,
  nodeBusy,
  currentLocale,
  onProbe,
  onAutoSelect,
  onSelect,
}: {
  nodes: NodesResult | null;
  nodeBusy: string | null;
  currentLocale: Locale;
  onProbe: () => void;
  onAutoSelect: () => void;
  onSelect: (name: string) => void;
}) {
  const t = copy[currentLocale];
  const selected = nodes?.selected ?? null;
  const options = nodes?.nodes ?? [];
  const selectedResult = selected ? nodes?.results[selected] : undefined;
  const busy = Boolean(nodeBusy);

  return (
    <>
      <PanelSectionRow>
        <div style={{ display: "grid", gap: "8px" }}>
          <div style={{ display: "flex", alignItems: "center", gap: "10px" }}>
            <ControlIcon name="nodes" />
            <div style={{ minWidth: 0, flex: 1 }}>
              <div style={{ color: "#8f98a0", fontSize: "12px", fontWeight: 700 }}>
                {t.currentNode}
              </div>
              <div
                style={{
                  color: "#f1f3f5",
                  fontSize: "15px",
                  fontWeight: 750,
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                }}
              >
                {selected ?? t.noNodeSelected}
              </div>
            </div>
            <div style={{ color: nodeDelayColor(selectedResult), fontSize: "13px", fontWeight: 700 }}>
              {nodeDelayLabel(selectedResult, currentLocale)}
            </div>
          </div>
          <div style={{ color: "#8f98a0", fontSize: "12px" }}>
            {nodes?.error
              ? userFacingMessage(nodes.error, currentLocale)
              : `${t.nodeCount(options.length)} · ${t.nodeProbeDesc}`}
          </div>
        </div>
      </PanelSectionRow>

      <PanelSectionRow>
        <ButtonItem
          layout="inline"
          icon={<ControlIcon name="test" />}
          label={t.probeNodes}
          description={t.nodeProbeDesc}
          disabled={busy || options.length === 0}
          onClick={onProbe}
        >
          {nodeBusy === "probe" ? t.probingNodes : t.probeNodes}
        </ButtonItem>
      </PanelSectionRow>

      <PanelSectionRow>
        <ButtonItem
          layout="inline"
          icon={<ControlIcon name="status" />}
          label={t.autoSelect}
          description={t.autoSelectDesc}
          disabled={busy || options.length === 0}
          onClick={onAutoSelect}
        >
          {nodeBusy === "auto" ? t.probingNodes : t.autoSelect}
        </ButtonItem>
      </PanelSectionRow>

      {options.length === 0 ? (
        <PanelSectionRow>
          <div style={{ color: "#8f98a0", fontSize: "13px" }}>{t.noNodes}</div>
        </PanelSectionRow>
      ) : null}

      {options.map((node) => {
        const isSelected = node.name === selected;
        const result = nodes?.results[node.name];
        const description = result?.error
          ? userFacingMessage(result.error, currentLocale)
          : `${node.type ?? "proxy"} · ${nodeDelayLabel(result, currentLocale)}`;
        const switchingThis = nodeBusy === node.name;

        return (
          <PanelSectionRow key={node.name}>
            <ButtonItem
              layout="inline"
              icon={<NodeStatusDot result={result} selected={isSelected} />}
              label={node.name}
              description={description}
              disabled={busy && !switchingThis}
              onClick={() => onSelect(node.name)}
            >
              {switchingThis ? t.switchingNode : isSelected ? t.selectedNode : t.useNode}
            </ButtonItem>
          </PanelSectionRow>
        );
      })}
    </>
  );
}

function Diagnostics({
  status,
  currentLocale,
}: {
  status: HelperStatus | null;
  currentLocale: Locale;
}) {
  if (!status) {
    return null;
  }

  const t = copy[currentLocale];
  const state = t.status[status.protection] ?? t.status.off;
  const backendRunning = status.active || status.tun_active || status.dns_active;

  return (
    <div style={{ display: "grid", gap: "6px", fontSize: "12px" }}>
      <DetailRow
        label={t.controlMode}
        value={status.install_mode === "system" ? t.gameModeService : status.install_mode}
      />
      <DetailRow label={t.protection} value={state.title} />
      <DetailRow label={t.backend} value={backendRunning ? t.running : t.stopped} />
      <DetailRow label={t.tunnel} value={status.tun_active ? status.tun_name : t.off} />
      <DetailRow
        label={t.address}
        value={status.tun_addresses.length ? status.tun_addresses.join(", ") : t.none}
      />
      <DetailRow
        label={t.nameServer}
        value={status.dns_servers.length ? status.dns_servers.join(", ") : t.none}
      />
      <DetailRow label={t.plugin} value={status.plugin_version} />
    </div>
  );
}

function LogView({ logs }: { logs: string | null }) {
  if (!logs) {
    return null;
  }

  return (
    <pre
      style={{
        maxHeight: "260px",
        overflow: "auto",
        whiteSpace: "pre-wrap",
        wordBreak: "break-word",
        fontSize: "11px",
        lineHeight: 1.35,
        padding: "10px",
        borderRadius: "6px",
        background: "rgba(0, 0, 0, 0.28)",
      }}
    >
      {logs}
    </pre>
  );
}

function statsFromConnections(snapshot: ConnectionSnapshot): RuntimeStats {
  let tcpConnections = 0;
  let udpConnections = 0;
  for (const connection of Object.values(snapshot.connections ?? {})) {
    if ((connection.protocol ?? "").toLowerCase() === "udp") {
      udpConnections += 1;
    } else {
      tcpConnections += 1;
    }
  }
  return {
    timestamp_ms: Date.now(),
    active_connections: Object.keys(snapshot.connections ?? {}).length,
    tcp_connections: tcpConnections,
    udp_connections: udpConnections,
    total_upload: snapshot.total_upload ?? 0,
    total_download: snapshot.total_download ?? 0,
    rx_bytes: snapshot.total_download ?? 0,
    tx_bytes: snapshot.total_upload ?? 0,
    rx_packets: 0,
    tx_packets: 0,
  };
}

function Content() {
  const currentLocale = locale();
  const t = copy[currentLocale];
  const [engineStatus, setEngineStatus] = useState<EngineStatus>({ status: "Connecting" });
  const [daemonConnected, setDaemonConnected] = useState(false);
  const [status, setStatus] = useState<HelperStatus | null>(null);
  const [stats, setStats] = useState<RuntimeStats | null>(null);
  const [nodes, setNodes] = useState<NodesResult | null>(null);
  const [trafficHistory, setTrafficHistory] = useState<TrafficPoint[]>([]);
  const [logs, setLogs] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [nodeBusy, setNodeBusy] = useState<string | null>(null);
  const lastStats = useRef<RuntimeStats | null>(null);
  const logsOpen = useRef(false);

  useEffect(() => {
    setStatus(statusFromEngine(engineStatus, daemonConnected));
  }, [engineStatus, daemonConnected]);

  const refreshNodes = async () => {
    const next = await loadNodes(nodes);
    setDaemonConnected(true);
    setNodes((current) => ({
      ...next,
      results: { ...(current?.results ?? {}), ...(next.results ?? {}) },
    }));
    if (next.error) {
      toaster.toast({ title: t.nodeLoadFailed, body: userFacingMessage(next.error, currentLocale) });
    }
  };

  const applyConnectionSnapshot = (snapshot: ConnectionSnapshot) => {
    const next = statsFromConnections(snapshot ?? EMPTY_CONNECTIONS);
    setStats(next);
    const previous = lastStats.current;
    lastStats.current = next;
    setTrafficHistory((history) => {
      const elapsed = previous ? Math.max(next.timestamp_ms - previous.timestamp_ms, 1) : 1000;
      const downRate = previous
        ? Math.max(0, ((next.rx_bytes - previous.rx_bytes) * 1000) / elapsed)
        : 0;
      const upRate = previous
        ? Math.max(0, ((next.tx_bytes - previous.tx_bytes) * 1000) / elapsed)
        : 0;
      return [...history.slice(-27), { id: next.timestamp_ms, downRate, upRate }];
    });
  };

  const runCommandAction = async (
    name: string,
    action: () => Promise<void>,
    optimistic: EngineStatus,
    successMessage: string,
  ) => {
    setBusy(name);
    try {
      setEngineStatus(optimistic);
      await action();
      toaster.toast({ title: "Rabbit Digger Pro", body: successMessage });
      await refreshNodes();
    } catch (error) {
      toaster.toast({ title: "Rabbit Digger Pro", body: userFacingMessage(String(error), currentLocale) });
    } finally {
      setBusy(null);
    }
  };

  const onToggleLogs = async () => {
    if (logs) {
      logsOpen.current = false;
      setLogs(null);
      return;
    }
    setBusy("logs");
    try {
      const result = await getLogs();
      logsOpen.current = true;
      setLogs(result.logs || t.noLogs);
    } catch (error) {
      toaster.toast({ title: t.logsFailed, body: userFacingMessage(String(error), currentLocale) });
    } finally {
      setBusy(null);
    }
  };

  const probeNodeList = async (targetNames?: string[]) => {
    const targets = targetNames ?? nodes?.nodes.map((node) => node.name) ?? [];
    if (targets.length === 0) {
      return null;
    }
    setNodes((current) => {
      if (!current) {
        return current;
      }
      const testing = Object.fromEntries(
        targets.map((name) => [
          name,
          {
            name,
            status: "testing" as const,
            connect: null,
            response: null,
            latency: null,
            error: null,
            tested_at: Date.now(),
          },
        ]),
      );
      return { ...current, results: { ...current.results, ...testing } };
    });

    const queue = [...new Set(targets)];
    const results: Record<string, NodeDelayResult> = {};
    const workerCount = Math.min(6, queue.length);
    await Promise.all(
      Array.from({ length: workerCount }, async () => {
        while (queue.length > 0) {
          const name = queue.shift();
          if (!name) {
            return;
          }
          try {
            const delay = await rpc<{ connect: number; response: number } | null>("net.delay", {
              net_name: name,
              url: NODE_DELAY_URL,
              timeout: NODE_DELAY_TIMEOUT_MS,
            });
            results[name] = {
              name,
              status: delay ? "ok" : "timeout",
              connect: delay?.connect ?? null,
              response: delay?.response ?? null,
              latency: delay?.response ?? delay?.connect ?? null,
              error: null,
              tested_at: Date.now(),
            };
          } catch (error) {
            const message = String(error);
            results[name] = {
              name,
              status: /timeout|elapsed/i.test(message) ? "timeout" : "error",
              connect: null,
              response: null,
              latency: null,
              error: message,
              tested_at: Date.now(),
            };
          }
        }
      }),
    );

    const next = nodes ? { ...nodes, results: { ...nodes.results, ...results } } : null;
    if (next) {
      setNodes(next);
    }
    return next;
  };

  const onProbeNodes = async () => {
    setNodeBusy("probe");
    try {
      await probeNodeList();
    } catch (error) {
      toaster.toast({ title: t.nodeLoadFailed, body: userFacingMessage(String(error), currentLocale) });
    } finally {
      setNodeBusy(null);
    }
  };

  const onSelectNode = async (name: string) => {
    if (!nodes || nodes.selected === name) {
      return;
    }
    setNodeBusy(name);
    try {
      await rpc<null>("net.select", { net_name: nodes.net_name, selected: name });
      setNodes((current) => current
        ? {
            ...current,
            selected: name,
            nodes: current.nodes.map((node) => ({ ...node, selected: node.name === name })),
          }
        : current,
      );
      toaster.toast({ title: "Rabbit Digger Pro", body: t.nodeSwitchOk(name) });
    } catch (error) {
      toaster.toast({ title: "Rabbit Digger Pro", body: userFacingMessage(String(error), currentLocale) });
    } finally {
      setNodeBusy(null);
    }
  };

  const onAutoSelectNode = async () => {
    setNodeBusy("auto");
    try {
      const tested = await probeNodeList();
      const fastest = findFastestNode(tested ?? nodes);
      if (!fastest) {
        toaster.toast({ title: "Rabbit Digger Pro", body: t.noReachableNode });
        return;
      }
      await onSelectNode(fastest.name);
      const latency = formatLatency(fastest.latency) ?? "";
      toaster.toast({
        title: "Rabbit Digger Pro",
        body: t.fastestSelected(fastest.name, latency),
      });
    } catch (error) {
      toaster.toast({ title: "Rabbit Digger Pro", body: userFacingMessage(String(error), currentLocale) });
    } finally {
      setNodeBusy(null);
    }
  };

  const onUpdate = async () => {
    setBusy("update");
    try {
      const result = await rpc<UpdateResult>("update.apply", { apply: true });
      if (result.ok) {
        toaster.toast({
          title: t.updatedTitle,
          body: result.needs_reload
            ? t.updatedNeedsReload
            : t.installedVersion(result.version ?? t.latestVersion),
        });
      } else {
        toaster.toast({
          title: t.updateFailed,
          body: result.error ? userFacingMessage(result.error, currentLocale) : t.unknownError,
        });
      }
    } catch (error) {
      toaster.toast({ title: t.updateFailed, body: userFacingMessage(String(error), currentLocale) });
    } finally {
      setBusy(null);
    }
  };

  useEffect(() => {
    const unsubEngine = subscribe("engine.events", {}, (payload) => {
      setDaemonConnected(true);
      const event = payload as ServerEvent;
      if (event.event === "StatusChanged" && event.status) {
        setEngineStatus(event.status);
      }
      if (event.event === "ConfigChanged") {
        void refreshNodes();
      }
    });

    const unsubConnections = subscribe("connections", { patch: false }, (payload) => {
      setDaemonConnected(true);
      applyConnectionSnapshot(payload as ConnectionSnapshot);
    });

    const unsubLogs = subscribe("logs", {}, (payload) => {
      if (!logsOpen.current) {
        return;
      }
      const line = typeof payload === "string" ? payload : JSON.stringify(payload);
      setLogs((current) => `${current ? `${current}\n` : ""}${line}`.split("\n").slice(-120).join("\n"));
    });

    void refreshNodes().catch((error) => {
      setDaemonConnected(false);
      setEngineStatus({ status: "Connecting", message: String(error) });
    });

    return () => {
      unsubEngine();
      unsubConnections();
      unsubLogs();
    };
  }, []);

  const isBusy = busy !== null;
  const statusReady = status !== null;
  const protectionOn = Boolean(status?.engine_active);

  return (
    <>
      <PanelSection title={t.sectionGameProxy}>
        <PanelSectionRow>
          <RuntimePanel
            status={status}
            stats={stats}
            history={trafficHistory}
            currentLocale={currentLocale}
          />
        </PanelSectionRow>
        {isBusy ? (
          <PanelSectionRow>
            <Spinner />
          </PanelSectionRow>
        ) : null}
        <PanelSectionRow>
          <ToggleField
            icon={<ControlIcon name="proxy" />}
            label={t.gameProxy}
            description={t.gameProxyDesc}
            checked={protectionOn}
            disabled={isBusy || !statusReady}
            onChange={(checked) =>
              runCommandAction(
                checked ? "start" : "stop",
                checked ? applyLastConfig : () => rpc<{ ok: boolean }>("engine.stop").then(() => undefined),
                { status: checked ? "Starting" : "Stopping" },
                checked ? t.startOk : t.stopOk,
              )
            }
          />
        </PanelSectionRow>
        <PanelSectionRow>
          <ButtonItem
            layout="inline"
            icon={<ControlIcon name="restart" />}
            label={t.restartLabel}
            description={t.restartDesc}
            disabled={isBusy || !protectionOn}
            onClick={() => runCommandAction(
              "restart",
              async () => {
                await rpc<{ ok: boolean }>("engine.stop").catch(() => undefined);
                await applyLastConfig();
              },
              { status: "Starting" },
              t.restartOk,
            )}
          >
            {t.restartButton}
          </ButtonItem>
        </PanelSectionRow>
      </PanelSection>

      <PanelSection title={t.sectionNodes}>
        <NodesPanel
          nodes={nodes}
          nodeBusy={nodeBusy}
          currentLocale={currentLocale}
          onProbe={onProbeNodes}
          onAutoSelect={onAutoSelectNode}
          onSelect={onSelectNode}
        />
      </PanelSection>

      <PanelSection title={t.sectionUpdates}>
        <PanelSectionRow>
          <ButtonItem
            layout="inline"
            icon={<ControlIcon name="update" />}
            label={t.updateLabel}
            description={t.updateDesc}
            disabled={isBusy}
            onClick={onUpdate}
          >
            {t.updateButton}
          </ButtonItem>
        </PanelSectionRow>
      </PanelSection>

      <PanelSection title={t.sectionDetails}>
        <PanelSectionRow>
          <Diagnostics status={status} currentLocale={currentLocale} />
        </PanelSectionRow>
        <PanelSectionRow>
          <ButtonItem
            layout="inline"
            icon={<ControlIcon name="status" />}
            label={t.statusLabel}
            description={t.statusDesc}
            disabled={isBusy}
            onClick={() => void refreshNodes()}
          >
            {t.refresh}
          </ButtonItem>
        </PanelSectionRow>
        <PanelSectionRow>
          <ButtonItem
            layout="inline"
            icon={<ControlIcon name="logs" />}
            label={t.logsLabel}
            description={t.logsDesc}
            disabled={isBusy}
            onClick={onToggleLogs}
          >
            {logs ? t.hide : t.show}
          </ButtonItem>
        </PanelSectionRow>
        {logs ? (
          <PanelSectionRow>
            <LogView logs={logs} />
          </PanelSectionRow>
        ) : null}
      </PanelSection>
    </>
  );
}

export default definePlugin(() => {
  return {
    name: "Rabbit Digger Pro",
    titleView: <div className={staticClasses.Title}>Rabbit Digger Pro</div>,
    content: <Content />,
    icon: <RabbitDiggerIcon />,
  };
});
