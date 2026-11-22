//! All application state and the JSON-RPC plumbing that feeds it.
//!
//! Views never talk to the socket: they read this entity and call the small
//! set of methods below, which send a request and let the reply land here.

use std::collections::{HashMap, HashSet, VecDeque};

use futures::StreamExt;
use gpui::{Context, Task};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::api::{self, BackendEvent, Connection};

pub const MAX_LOGS: usize = 500;
pub const TRAFFIC_POINTS: usize = 48;
pub const SETTINGS_KEY: &str = "daemon/settings";
pub const DELAY_TEST_URL: &str = "http://www.gstatic.com/generate_204";

#[derive(Clone, Debug, Default)]
pub struct ConnRow {
    pub id: String,
    pub protocol: String,
    pub host: String,
    pub route: String,
    pub source: String,
    pub upload: u64,
    pub download: u64,
    pub started: u64,
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub level: String,
    pub time: String,
    pub message: String,
}

#[derive(Clone, Debug, Default)]
pub struct ServerRow {
    pub name: String,
    pub kind: String,
    pub status: String,
    pub message: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct EngineStatus {
    pub state: String,
    pub message: Option<String>,
    pub servers: Vec<ServerRow>,
}

/// One subscription entry, matching the shape the React UI stored under
/// `daemon/settings` so an existing install keeps its data.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Subscription {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub url: String,
    #[serde(default = "default_interval")]
    pub interval: u64,
    #[serde(default)]
    pub updated_at: Option<String>,
}

impl Default for Subscription {
    fn default() -> Self {
        Self {
            name: String::new(),
            url: String::new(),
            interval: default_interval(),
            updated_at: None,
        }
    }
}

fn default_interval() -> u64 {
    60
}

fn default_port() -> u16 {
    10800
}

fn default_bootstrap_dns() -> String {
    "cloudflare".to_string()
}

fn default_tun_dns_mode() -> String {
    "fake-ip".to_string()
}

fn default_mtu() -> u16 {
    1400
}

/// The form behind the settings page, kept in userdata as JSON. Defaults mirror
/// the ones the React settings page used.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DaemonSettings {
    pub subscriptions: Vec<Subscription>,
    pub active_index: i64,
    pub port: u16,
    pub bootstrap_dns: String,
    pub tun_enabled: bool,
    pub tun_dns_mode: String,
    pub tun_ip: String,
    pub tun_mtu: u16,
}

impl Default for DaemonSettings {
    fn default() -> Self {
        Self {
            subscriptions: Vec::new(),
            active_index: 0,
            port: default_port(),
            bootstrap_dns: default_bootstrap_dns(),
            tun_enabled: false,
            tun_dns_mode: default_tun_dns_mode(),
            tun_ip: String::new(),
            tun_mtu: default_mtu(),
        }
    }
}

impl DaemonSettings {
    pub fn active_subscription(&self) -> Option<&Subscription> {
        if self.active_index < 0 {
            return None;
        }
        self.subscriptions.get(self.active_index as usize)
    }

    /// The config the engine should run, built the same way the React page did:
    /// a clash import for the active subscription, a mixed listener, and an
    /// optional TUN sink.
    pub fn to_config_text(&self) -> String {
        let mut net = Map::new();
        let mut local = json!({ "type": "local" });
        let mut bootstrap_out = json!({ "type": "local" });
        if self.tun_enabled {
            // Host-side local nets bypass the TUN route; without the mark the
            // engine's own bootstrap traffic loops back into the tunnel.
            local["mark"] = json!(255);
            bootstrap_out["mark"] = json!(255);
            // Windows has no SO_MARK; force these sockets onto the physical
            // interface so proxy/DNS/direct traffic cannot enter the TUN.
            local["bind_default_interface"] = json!(true);
            bootstrap_out["bind_default_interface"] = json!(true);
        }
        net.insert("local".to_string(), local);
        net.insert("rdp_bootstrap_out".to_string(), bootstrap_out);
        net.insert(
            "rdp_bootstrap_dns".to_string(),
            json!({
                "type": "dns",
                "server": self.bootstrap_dns,
                "net": "rdp_bootstrap_out",
            }),
        );
        let mut node_out = json!({ "type": "local", "lookup_host": "rdp_bootstrap_dns" });
        if self.tun_enabled {
            node_out["mark"] = json!(255);
            node_out["bind_default_interface"] = json!(true);
        }
        net.insert("rdp_node_out".to_string(), node_out);

        let active = self.active_subscription();
        let outbound = if active.is_some() { "rule" } else { "proxy" };

        let mut imports = Vec::new();
        if let Some(subscription) = active {
            imports.push(json!({
                "type": "clash",
                "source": {
                    "poll": {
                        "url": subscription.url,
                        "interval": subscription.interval.saturating_mul(60),
                    }
                },
                "rule_name": "rule",
                "net": "rdp_node_out",
            }));
        } else {
            net.insert(
                "proxy".to_string(),
                json!({ "type": "alias", "net": "local" }),
            );
        }

        let mut server = Map::new();
        if self.tun_enabled {
            net.insert(
                "resolve".to_string(),
                json!({
                    "type": "resolve",
                    "net": outbound,
                    "resolve_net": "local",
                    "ipv6": false,
                }),
            );
            let mut tun = json!({
                "type": "tun",
                "device": "utun100",
                "dns_mode": self.tun_dns_mode,
                "net": "resolve",
                "fwmark": 255,
                "bypass_ips": [],
            });
            if !self.tun_ip.is_empty() {
                tun["ip_addr"] = json!(self.tun_ip);
            }
            if self.tun_mtu > 0 {
                tun["mtu"] = json!(self.tun_mtu);
            }
            server.insert("tun".to_string(), tun);
        }

        server.insert(
            "mixed".to_string(),
            json!({
                "type": "http+socks5",
                "bind": format!("127.0.0.1:{}", self.port),
                "net": outbound,
                "listen": "local",
            }),
        );

        json!({
            "id": "daemon",
            "net": net,
            "server": server,
            "import": imports,
        })
        .to_string()
    }
}

#[derive(Clone, Debug)]
pub struct Notice {
    pub ok: bool,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct AutoNodeStatus {
    pub selected: String,
}

#[derive(Clone, Debug)]
pub enum DelayState {
    Testing,
    Done { response: u64 },
    Timeout,
    Error,
}

#[derive(Clone, Debug)]
enum Pending {
    Config,
    LogsTail,
    SelectNet,
    CloseConnection,
    CloseAll,
    EngineStop,
    SettingsGet,
    TunSuggest,
    SettingsPut { apply: bool },
    ApplyConfig,
    NetDelay(String),
    NetStatus(String),
    Other,
}

pub struct Store {
    pub endpoint: String,
    pub connected: bool,
    pub config: Option<Value>,
    pub engine: EngineStatus,
    pub rows: Vec<ConnRow>,
    pub total_up: u64,
    pub total_down: u64,
    pub up_rate: f64,
    pub down_rate: f64,
    pub traffic: VecDeque<(f64, f64)>,
    pub logs: VecDeque<LogLine>,
    pub error: Option<String>,

    // --- settings page ---------------------------------------------------
    pub settings: DaemonSettings,
    pub settings_loaded: bool,
    pub settings_busy: bool,
    pub settings_notice: Option<Notice>,
    pub tun_ip_suggestion: Option<String>,
    /// Delay results of the node page, keyed by candidate name.
    pub delays: HashMap<String, DelayState>,
    /// Runtime selections of url-test/fallback groups, keyed by net name.
    pub auto_selected: HashMap<String, AutoNodeStatus>,
    /// Label of the running delay test, `None` when idle.
    pub delay_scope: Option<String>,
    delay_pending: usize,

    conn: Option<Connection>,
    next_id: u64,
    pending: HashMap<u64, Pending>,
    last_totals: Option<(u64, u64)>,
    pump: Option<Task<()>>,
    retry: Option<Task<()>>,
    status_timer: Option<Task<()>>,
}

impl Store {
    pub fn new(endpoint: String, cx: &mut Context<Self>) -> Self {
        let mut store = Self {
            endpoint,
            connected: false,
            config: None,
            engine: EngineStatus {
                state: "Connecting".to_string(),
                ..Default::default()
            },
            rows: Vec::new(),
            total_up: 0,
            total_down: 0,
            up_rate: 0.0,
            down_rate: 0.0,
            traffic: VecDeque::new(),
            logs: VecDeque::new(),
            error: None,
            settings: DaemonSettings::default(),
            settings_loaded: false,
            settings_busy: false,
            settings_notice: None,
            tun_ip_suggestion: None,
            delays: HashMap::new(),
            auto_selected: HashMap::new(),
            delay_scope: None,
            delay_pending: 0,
            conn: None,
            next_id: 1,
            pending: HashMap::new(),
            last_totals: None,
            pump: None,
            retry: None,
            status_timer: None,
        };
        store.connect(cx);
        store.status_timer = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(30))
                .await;
            let keep_going = this
                .update(cx, |store, cx| {
                    store.request_auto_statuses();
                    cx.notify();
                })
                .is_ok();
            if !keep_going {
                break;
            }
        }));
        store
    }

    fn connect(&mut self, cx: &mut Context<Self>) {
        let (conn, mut events) = api::connect(api::ws_url(&self.endpoint));
        self.conn = Some(conn);

        self.pump = Some(cx.spawn(async move |this, cx| {
            while let Some(event) = events.next().await {
                let keep_going = this.update(cx, |store, cx| store.handle(event, cx)).is_ok();
                if !keep_going {
                    break;
                }
            }
        }));
    }

    fn handle(&mut self, event: BackendEvent, cx: &mut Context<Self>) -> bool {
        match event {
            BackendEvent::Connected => {
                self.connected = true;
                self.error = None;
                self.retry = None;
                self.request_config();
                self.request_settings();
                self.request_logs(MAX_LOGS as u64);
                self.subscribe("engine.events", json!({}));
                self.subscribe("connections", json!({"patch": false}));
                self.subscribe("logs", json!({}));
                cx.notify();
            }
            BackendEvent::Closed(reason) => {
                self.connected = false;
                self.conn = None;
                // Replies for in-flight requests will never arrive.
                self.pending.clear();
                if self.engine.state != "Stopped" {
                    self.error = Some(format!("与引擎的连接已断开：{reason}"));
                }
                self.schedule_reconnect(cx);
                cx.notify();
            }
            BackendEvent::Text(text) => {
                self.handle_message(&text, cx);
            }
        }
        true
    }

    fn schedule_reconnect(&mut self, cx: &mut Context<Self>) {
        if self.retry.is_some() {
            return;
        }
        self.retry = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(2))
                .await;
            let _ = this.update(cx, |store, cx| {
                store.retry = None;
                store.connect(cx);
                cx.notify();
            });
        }));
    }

    fn handle_message(&mut self, text: &str, cx: &mut Context<Self>) {
        let Ok(value) = serde_json::from_str::<Value>(text) else {
            return;
        };

        if value.get("method").and_then(Value::as_str) == Some("rpc.subscription") {
            let params = value.get("params").cloned().unwrap_or(Value::Null);
            let topic = params.get("topic").and_then(Value::as_str).unwrap_or("");
            let payload = params.get("payload").cloned().unwrap_or(Value::Null);
            self.handle_subscription(topic, payload, cx);
            return;
        }

        let Some(id) = value.get("id").and_then(Value::as_u64) else {
            return;
        };
        let Some(kind) = self.pending.remove(&id) else {
            return;
        };

        if let Some(error) = value.get("error") {
            // A daemon started without a log file answers `logs.tail` with an
            // error; that is a normal deployment, not something to surface.
            if matches!(&kind, Pending::LogsTail) {
                return;
            }
            // No settings stored yet, or an engine without TUN support: both
            // are normal, so the settings page keeps its defaults.
            if matches!(&kind, Pending::SettingsGet | Pending::TunSuggest) {
                if matches!(&kind, Pending::SettingsGet) {
                    self.settings_loaded = true;
                    if self.settings.tun_ip.is_empty() {
                        if let Some(ip) = &self.tun_ip_suggestion {
                            self.settings.tun_ip = ip.clone();
                        }
                    }
                    cx.notify();
                }
                return;
            }
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("RPC 调用失败")
                .to_string();
            match kind {
                Pending::SettingsPut { .. } | Pending::ApplyConfig => {
                    self.settings_busy = false;
                    self.settings_notice = Some(Notice {
                        ok: false,
                        text: message,
                    });
                }
                Pending::NetDelay(candidate) => {
                    self.finish_delay(&candidate, DelayState::Error);
                }
                Pending::NetStatus(name) => {
                    // A stopped or non-auto net has no status; that is normal.
                    self.auto_selected.remove(&name);
                }
                _ => self.error = Some(message),
            }
            cx.notify();
            return;
        }

        let result = value.get("result").cloned().unwrap_or(Value::Null);
        match kind {
            Pending::Config => {
                self.config = if result.is_null() { None } else { Some(result) };
                self.request_auto_statuses();
                cx.notify();
            }
            Pending::LogsTail => {
                if let Some(entries) = result.as_array() {
                    for entry in entries {
                        if let Some(line) = parse_log_value(entry) {
                            push_log(&mut self.logs, line);
                        }
                    }
                    cx.notify();
                }
            }
            Pending::SelectNet | Pending::EngineStop => {
                self.request_config();
                cx.notify();
            }
            Pending::CloseConnection | Pending::CloseAll => {
                cx.notify();
            }
            Pending::SettingsGet => {
                self.apply_stored_settings(&result);
                cx.notify();
            }
            Pending::TunSuggest => {
                self.tun_ip_suggestion = result
                    .get("ip")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .filter(|ip| !ip.is_empty());
                if self.settings_loaded && self.settings.tun_ip.is_empty() {
                    if let Some(ip) = &self.tun_ip_suggestion {
                        self.settings.tun_ip = ip.clone();
                    }
                }
                cx.notify();
            }
            Pending::SettingsPut { apply } => {
                if apply {
                    self.send(
                        "config.apply",
                        json!({ "text": self.settings.to_config_text() }),
                        Pending::ApplyConfig,
                    );
                } else {
                    self.settings_busy = false;
                    self.settings_notice = Some(Notice {
                        ok: true,
                        text: "设置已保存".to_string(),
                    });
                }
                cx.notify();
            }
            Pending::ApplyConfig => {
                self.settings_busy = false;
                self.settings_notice = Some(Notice {
                    ok: true,
                    text: "配置已应用".to_string(),
                });
                self.request_config();
                cx.notify();
            }
            Pending::NetDelay(candidate) => {
                let state = if result.is_null() {
                    DelayState::Timeout
                } else {
                    DelayState::Done {
                        response: result.get("response").and_then(Value::as_u64).unwrap_or(0),
                    }
                };
                self.finish_delay(&candidate, state);
                cx.notify();
            }
            Pending::NetStatus(name) => {
                let kind = result.get("type").and_then(Value::as_str);
                if matches!(kind, Some("url-test") | Some("fallback")) {
                    self.auto_selected.insert(
                        name,
                        AutoNodeStatus {
                            selected: result
                                .get("selected")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                        },
                    );
                } else {
                    self.auto_selected.remove(&name);
                }
                cx.notify();
            }
            Pending::Other => {}
        }
    }

    fn apply_stored_settings(&mut self, result: &Value) {
        let content = result.get("content").and_then(Value::as_str);
        if let Some(content) = content {
            if let Ok(settings) = serde_json::from_str::<DaemonSettings>(content) {
                self.settings = settings;
            }
        }
        if self.settings.tun_ip.is_empty() {
            if let Some(ip) = &self.tun_ip_suggestion {
                self.settings.tun_ip = ip.clone();
            }
        }
        self.settings_loaded = true;
    }

    fn finish_delay(&mut self, candidate: &str, state: DelayState) {
        self.delays.insert(candidate.to_string(), state);
        self.delay_pending = self.delay_pending.saturating_sub(1);
        if self.delay_pending == 0 {
            self.delay_scope = None;
        }
    }

    fn handle_subscription(&mut self, topic: &str, payload: Value, cx: &mut Context<Self>) {
        match topic {
            "engine.events" => {
                let event = payload.get("event").and_then(Value::as_str).unwrap_or("");
                match event {
                    "StatusChanged" => {
                        if let Some(status) = payload.get("status") {
                            self.engine = parse_engine_status(status);
                            cx.notify();
                        }
                    }
                    "ConfigChanged" => self.request_config(),
                    _ => {}
                }
            }
            "connections" => {
                let snapshot = match payload.get("full") {
                    Some(full) => full.clone(),
                    None => payload,
                };
                self.apply_snapshot(&snapshot);
                cx.notify();
            }
            "logs" => {
                let chunk = match payload {
                    Value::String(text) => text,
                    other => other.to_string(),
                };
                for raw in chunk.lines() {
                    if let Some(line) = parse_log_line(raw) {
                        push_log(&mut self.logs, line);
                    }
                }
                cx.notify();
            }
            _ => {}
        }
    }

    fn apply_snapshot(&mut self, snapshot: &Value) {
        let total_up = snapshot
            .get("total_upload")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let total_down = snapshot
            .get("total_download")
            .and_then(Value::as_u64)
            .unwrap_or(0);

        if let Some((prev_up, prev_down)) = self.last_totals {
            let up = total_up.saturating_sub(prev_up) as f64;
            let down = total_down.saturating_sub(prev_down) as f64;
            self.up_rate = up;
            self.down_rate = down;
            self.traffic.push_back((up, down));
            while self.traffic.len() > TRAFFIC_POINTS {
                self.traffic.pop_front();
            }
        }
        self.last_totals = Some((total_up, total_down));
        self.total_up = total_up;
        self.total_down = total_down;

        self.rows.clear();
        if let Some(map) = snapshot.get("connections").and_then(Value::as_object) {
            for (id, entry) in map {
                let ctx = entry.get("ctx").unwrap_or(&Value::Null);
                let host = ctx
                    .get("dest_domain")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| entry.get("addr").map(format_address))
                    .unwrap_or_else(|| "-".to_string());
                let route = ctx
                    .get("net_list")
                    .map(format_address)
                    .or_else(|| ctx.get("net").map(format_address))
                    .unwrap_or_else(|| "-".to_string());
                let source = ctx
                    .get("src_socket_addr")
                    .map(format_address)
                    .or_else(|| ctx.get("src").map(format_address))
                    .unwrap_or_default();

                self.rows.push(ConnRow {
                    id: id.clone(),
                    protocol: entry
                        .get("protocol")
                        .and_then(Value::as_str)
                        .unwrap_or("-")
                        .to_string(),
                    host,
                    route,
                    source,
                    upload: entry.get("upload").and_then(Value::as_u64).unwrap_or(0),
                    download: entry.get("download").and_then(Value::as_u64).unwrap_or(0),
                    started: entry.get("start_time").and_then(Value::as_u64).unwrap_or(0),
                });
            }
        }
        self.rows.sort_by(|a, b| b.started.cmp(&a.started));
    }

    // --- requests -------------------------------------------------------

    fn send(&mut self, method: &str, params: Value, kind: Pending) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        if let Some(conn) = &self.conn {
            self.pending.insert(id, kind);
            conn.send(payload.to_string());
        }
        id
    }

    pub fn request_config(&mut self) {
        self.send("config.get", json!({}), Pending::Config);
    }

    /// Asks each url-test/fallback group for the node traffic currently goes
    /// through. The call also refreshes the group if its interval elapsed.
    fn request_auto_statuses(&mut self) {
        let names: Vec<String> = self
            .config
            .as_ref()
            .and_then(|config| config.get("net"))
            .and_then(Value::as_object)
            .map(|nets| {
                nets.iter()
                    .filter(|(_, net)| {
                        matches!(
                            net.get("type").and_then(Value::as_str),
                            Some("url-test") | Some("fallback")
                        )
                    })
                    .map(|(name, _)| name.clone())
                    .collect()
            })
            .unwrap_or_default();

        self.auto_selected
            .retain(|name, _| names.iter().any(|current| current == name));
        for name in names {
            self.send(
                "net.status",
                json!({ "net_name": name }),
                Pending::NetStatus(name),
            );
        }
    }

    pub fn request_logs(&mut self, tail: u64) {
        self.send("logs.tail", json!({ "tail": tail }), Pending::LogsTail);
    }

    fn subscribe(&mut self, topic: &str, params: Value) {
        self.send(
            "rpc.subscribe",
            json!({ "topic": topic, "params": params }),
            Pending::Other,
        );
    }

    pub fn select_net(&mut self, net: &str, selected: &str) {
        self.send(
            "net.select",
            json!({ "net_name": net, "selected": selected }),
            Pending::SelectNet,
        );
    }

    // --- settings page ---------------------------------------------------

    /// Loads the settings form and the daemon's suggested TUN address. Both are
    /// best-effort: a fresh install simply keeps the defaults.
    pub fn request_settings(&mut self) {
        self.send(
            "userdata.get",
            json!({ "path": SETTINGS_KEY }),
            Pending::SettingsGet,
        );
        self.send("tun.suggestIp", json!({}), Pending::TunSuggest);
    }

    pub fn save_settings(&mut self) {
        self.settings_busy = true;
        self.settings_notice = None;
        let value = serde_json::to_string(&self.settings).unwrap_or_else(|_| "{}".to_string());
        self.send(
            "userdata.put",
            json!({ "path": SETTINGS_KEY, "value": value }),
            Pending::SettingsPut { apply: false },
        );
    }

    /// Saves the form and asks the engine to run it.
    pub fn apply_settings(&mut self) {
        self.settings_busy = true;
        self.settings_notice = None;
        let value = serde_json::to_string(&self.settings).unwrap_or_else(|_| "{}".to_string());
        self.send(
            "userdata.put",
            json!({ "path": SETTINGS_KEY, "value": value }),
            Pending::SettingsPut { apply: true },
        );
    }

    pub fn add_subscription(&mut self, url: String) {
        let url = url.trim().to_string();
        if url.is_empty() {
            return;
        }
        self.settings.subscriptions.push(Subscription {
            name: subscription_name(&url),
            url,
            ..Subscription::default()
        });
        self.save_settings();
    }

    pub fn remove_subscription(&mut self, index: usize) {
        if index >= self.settings.subscriptions.len() {
            return;
        }
        self.settings.subscriptions.remove(index);

        let active = self.settings.active_index;
        if active == index as i64 {
            self.settings.active_index = if self.settings.subscriptions.is_empty() {
                -1
            } else {
                0
            };
        } else if active > index as i64 {
            self.settings.active_index -= 1;
        }
        self.save_settings();
    }

    pub fn activate_subscription(&mut self, index: usize) {
        if index >= self.settings.subscriptions.len() {
            return;
        }
        self.settings.active_index = index as i64;
        self.apply_settings();
    }

    pub fn update_subscription_url(&mut self, index: usize, url: String) {
        let url = url.trim().to_string();
        if url.is_empty() || index >= self.settings.subscriptions.len() {
            return;
        }
        let subscription = &mut self.settings.subscriptions[index];
        subscription.url = url;
        subscription.name = subscription_name(&subscription.url);
        self.save_settings();
    }

    /// Re-applies the config so the poll import fetches the subscription again.
    pub fn refresh_subscription(&mut self, index: usize) {
        if index >= self.settings.subscriptions.len() {
            return;
        }
        self.settings.subscriptions[index].updated_at = None;
        if self.engine.state == "Running" {
            self.apply_settings();
        } else {
            self.save_settings();
        }
    }

    pub fn set_notice(&mut self, ok: bool, text: impl Into<String>) {
        self.settings_notice = Some(Notice {
            ok,
            text: text.into(),
        });
    }

    // --- node delay ------------------------------------------------------

    pub fn test_delays(&mut self, scope: &str, targets: Vec<(String, String)>) {
        if self.delay_scope.is_some() || targets.is_empty() {
            return;
        }
        self.delay_scope = Some(scope.to_string());
        let mut seen = HashSet::new();
        for (net_name, candidate) in targets {
            if !seen.insert(candidate.clone()) {
                continue;
            }
            self.delays.insert(candidate.clone(), DelayState::Testing);
            self.send(
                "net.delay",
                json!({
                    "net_name": net_name,
                    "url": DELAY_TEST_URL,
                    "timeout": 5000,
                }),
                Pending::NetDelay(candidate),
            );
            self.delay_pending += 1;
        }
    }

    pub fn close_connection(&mut self, uuid: &str) {
        self.send(
            "connection.close",
            json!({ "uuid": uuid }),
            Pending::CloseConnection,
        );
    }

    pub fn close_all_connections(&mut self) {
        self.send("connection.closeAll", json!({}), Pending::CloseAll);
    }

    pub fn engine_stop(&mut self) {
        self.send("engine.stop", json!({}), Pending::EngineStop);
    }

    pub fn clear_logs(&mut self) {
        self.logs.clear();
    }
}

fn push_log(logs: &mut VecDeque<LogLine>, line: LogLine) {
    logs.push_back(line);
    while logs.len() > MAX_LOGS {
        logs.pop_front();
    }
}

/// The React page used the URL's hostname as the default subscription name.
fn subscription_name(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or(url);
    rest.split(['/', '?', '#'])
        .next()
        .filter(|host| !host.is_empty())
        .unwrap_or(url)
        .to_string()
}

/// Renders an RPC `Address` (a string, a list of candidates, or an object such
/// as `{ "ip": "...", "port": 443 }`) the way the old UI did.
fn format_address(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .map(format_address)
            .collect::<Vec<_>>()
            .join(" / "),
        Value::Object(map) => {
            if let (Some(host), Some(port)) = (
                map.get("domain")
                    .or_else(|| map.get("ip"))
                    .or_else(|| map.get("host"))
                    .and_then(Value::as_str),
                map.get("port").and_then(Value::as_u64),
            ) {
                return format!("{host}:{port}");
            }
            map.iter()
                .map(|(key, nested)| format!("{key}={}", format_address(nested)))
                .collect::<Vec<_>>()
                .join(" ")
        }
        other => other.to_string(),
    }
}

/// Seconds since the Unix epoch, on both targets.
#[cfg(not(target_family = "wasm"))]
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(target_family = "wasm")]
pub fn now_secs() -> u64 {
    (js_sys::Date::now() / 1000.0) as u64
}

fn parse_engine_status(value: &Value) -> EngineStatus {
    let state = match value.get("status") {
        Some(Value::String(state)) => state.clone(),
        Some(other) => other.to_string(),
        None => "Unknown".to_string(),
    };
    let mut servers = Vec::new();
    if let Some(list) = value.get("servers").and_then(Value::as_array) {
        for server in list {
            servers.push(ServerRow {
                name: server
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("-")
                    .to_string(),
                kind: server
                    .get("server_type")
                    .and_then(Value::as_str)
                    .unwrap_or("-")
                    .to_string(),
                status: server
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("-")
                    .to_string(),
                message: server
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            });
        }
    }
    EngineStatus {
        state,
        message: value
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string),
        servers,
    }
}

fn parse_log_value(value: &Value) -> Option<LogLine> {
    let level = value
        .get("level")
        .and_then(Value::as_str)
        .unwrap_or("LOG")
        .to_string();
    let time = value
        .get("timestamp")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let message = value
        .get("fields")
        .and_then(|fields| fields.get("message"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string());
    Some(LogLine {
        level,
        time,
        message,
    })
}

/// The `logs` topic streams raw text lines, which are usually JSON objects with
/// the same shape `logs.tail` returns.
fn parse_log_line(raw: &str) -> Option<LogLine> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        if value.is_object() {
            return parse_log_value(&value);
        }
    }
    Some(LogLine {
        level: "LOG".to_string(),
        time: String::new(),
        message: trimmed.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_status_reads_state_and_servers() {
        let status = parse_engine_status(&json!({
            "status": "Running",
            "servers": [
                { "name": "socks5", "server_type": "socks5", "status": "Running" }
            ]
        }));

        assert_eq!(status.state, "Running");
        assert_eq!(status.servers.len(), 1);
        assert_eq!(status.servers[0].name, "socks5");
        assert_eq!(status.servers[0].status, "Running");
    }

    #[test]
    fn engine_status_keeps_unknown_shape_readable() {
        let status = parse_engine_status(&json!({ "status": { "kind": "Idle" } }));
        assert!(status.state.contains("Idle"));
        assert!(status.servers.is_empty());
    }

    #[test]
    fn log_values_carry_level_time_and_message() {
        let line = parse_log_value(&json!({
            "timestamp": "2026-11-22T01:02:03.000Z",
            "level": "INFO",
            "fields": { "message": "engine started" }
        }))
        .expect("a log object parses");

        assert_eq!(line.level, "INFO");
        assert_eq!(line.time, "2026-11-22T01:02:03.000Z");
        assert_eq!(line.message, "engine started");
    }

    #[test]
    fn log_lines_accept_json_and_plain_text() {
        let structured = parse_log_line(r#"{"level":"ERROR","fields":{"message":"boom"}}"#)
            .expect("json line parses");
        assert_eq!(structured.level, "ERROR");
        assert_eq!(structured.message, "boom");

        let plain = parse_log_line("a bare trailing line").expect("plain line parses");
        assert_eq!(plain.level, "LOG");
        assert_eq!(plain.message, "a bare trailing line");

        assert!(parse_log_line("   ").is_none());
    }

    #[test]
    fn log_queue_is_capped() {
        let mut logs = VecDeque::new();
        for index in 0..(MAX_LOGS + 10) {
            push_log(
                &mut logs,
                LogLine {
                    level: "LOG".to_string(),
                    time: String::new(),
                    message: index.to_string(),
                },
            );
        }

        assert_eq!(logs.len(), MAX_LOGS);
        assert_eq!(logs.front().unwrap().message, "10");
    }

    #[test]
    fn settings_from_the_react_ui_still_parse() {
        let settings: DaemonSettings = serde_json::from_str(
            r#"{
                "subscriptions": [
                    { "name": "example", "url": "https://example.com/sub", "interval": 30,
                      "updatedAt": "2026-01-01T00:00:00.000Z" }
                ],
                "activeIndex": 0,
                "port": 10801,
                "bootstrapDns": "google",
                "tunEnabled": true,
                "tunDnsMode": "redir-host",
                "tunIp": "10.0.0.1/24",
                "tunMtu": 1380
            }"#,
        )
        .expect("the old settings shape deserializes");

        assert_eq!(settings.port, 10801);
        assert_eq!(settings.bootstrap_dns, "google");
        assert!(settings.tun_enabled);
        assert_eq!(settings.tun_mtu, 1380);
        assert_eq!(settings.active_subscription().unwrap().name, "example");
    }

    #[test]
    fn config_text_uses_the_alias_without_a_subscription() {
        let settings = DaemonSettings::default();
        let config: Value = serde_json::from_str(&settings.to_config_text()).unwrap();

        assert_eq!(config["net"]["proxy"]["type"], "alias");
        assert_eq!(config["net"]["proxy"]["net"], "local");
        assert_eq!(config["server"]["mixed"]["bind"], "127.0.0.1:10800");
        assert_eq!(config["server"]["mixed"]["net"], "proxy");
        assert!(config["import"].as_array().unwrap().is_empty());
        assert!(config["server"].get("tun").is_none());
    }

    #[test]
    fn config_text_polls_the_active_subscription_and_adds_tun() {
        let mut settings = DaemonSettings::default();
        settings.tun_enabled = true;
        settings.tun_ip = "10.0.0.1/24".to_string();
        settings.subscriptions.push(Subscription {
            name: "example".to_string(),
            url: "https://example.com/sub".to_string(),
            interval: 15,
            updated_at: None,
        });
        settings.active_index = 0;

        let config: Value = serde_json::from_str(&settings.to_config_text()).unwrap();

        assert_eq!(config["import"][0]["source"]["poll"]["interval"], 900);
        assert_eq!(config["import"][0]["rule_name"], "rule");
        assert!(config["import"][0].get("select").is_none());
        assert_eq!(config["server"]["mixed"]["net"], "rule");
        assert_eq!(config["net"]["resolve"]["net"], "rule");
        assert_eq!(config["server"]["tun"]["dns_mode"], "fake-ip");
        assert_eq!(config["server"]["tun"]["ip_addr"], "10.0.0.1/24");
        assert_eq!(config["net"]["local"]["mark"], 255);
        assert_eq!(config["net"]["local"]["bind_default_interface"], true);
        assert_eq!(
            config["net"]["rdp_bootstrap_out"]["bind_default_interface"],
            true
        );
        assert_eq!(
            config["net"]["rdp_node_out"]["bind_default_interface"],
            true
        );
    }

    #[test]
    fn subscription_names_fall_back_to_the_host() {
        assert_eq!(
            subscription_name("https://sub.example.com/a/b?x=1"),
            "sub.example.com"
        );
        assert_eq!(subscription_name("not a url"), "not a url");
    }

    #[test]
    fn addresses_render_like_the_react_page() {
        assert_eq!(format_address(&json!("example.com")), "example.com");
        assert_eq!(
            format_address(&json!([{ "domain": "a.test", "port": 443 }, "b.test:80"])),
            "a.test:443 / b.test:80"
        );
        assert_eq!(
            format_address(&json!({ "ip": "10.0.0.2", "port": 1080 })),
            "10.0.0.2:1080"
        );
    }
}
