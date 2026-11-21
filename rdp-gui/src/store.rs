//! All application state and the JSON-RPC plumbing that feeds it.
//!
//! Views never talk to the socket: they read this entity and call the small
//! set of methods below, which send a request and let the reply land here.

use std::collections::{HashMap, VecDeque};

use futures::StreamExt;
use gpui::{Context, Task};
use serde_json::{json, Value};

use crate::api::{self, BackendEvent, Connection};

pub const MAX_LOGS: usize = 500;
pub const TRAFFIC_POINTS: usize = 48;

#[derive(Clone, Debug, Default)]
pub struct ConnRow {
    pub id: String,
    pub protocol: String,
    pub addr: String,
    pub upload: u64,
    pub download: u64,
    pub started: Option<String>,
    pub ctx: String,
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub level: String,
    pub time: String,
    pub message: String,
}

#[derive(Clone, Debug, Default)]
pub struct EngineStatus {
    pub state: String,
    pub message: Option<String>,
    pub servers: Vec<(String, String, String)>,
}

#[derive(Clone, Copy, Debug)]
enum Pending {
    Config,
    LogsTail,
    SelectNet,
    CloseConnection,
    CloseAll,
    EngineStop,
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

    conn: Option<Connection>,
    next_id: u64,
    pending: HashMap<u64, Pending>,
    last_totals: Option<(u64, u64)>,
    pump: Option<Task<()>>,
    retry: Option<Task<()>>,
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
            conn: None,
            next_id: 1,
            pending: HashMap::new(),
            last_totals: None,
            pump: None,
            retry: None,
        };
        store.connect(cx);
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
                self.request_logs(MAX_LOGS as u64);
                self.subscribe("engine.events", json!({}));
                self.subscribe("connections", json!({"patch": false}));
                self.subscribe("logs", json!({}));
                cx.notify();
            }
            BackendEvent::Closed(reason) => {
                self.connected = false;
                self.conn = None;
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
            if matches!(kind, Pending::LogsTail) {
                return;
            }
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("RPC 调用失败")
                .to_string();
            self.error = Some(message);
            cx.notify();
            return;
        }

        let result = value.get("result").cloned().unwrap_or(Value::Null);
        match kind {
            Pending::Config => {
                self.config = if result.is_null() { None } else { Some(result) };
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
            Pending::Other => {}
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
                self.rows.push(ConnRow {
                    id: id.clone(),
                    protocol: entry
                        .get("protocol")
                        .and_then(Value::as_str)
                        .unwrap_or("-")
                        .to_string(),
                    addr: entry
                        .get("addr")
                        .map(|addr| addr.to_string().trim_matches('"').to_string())
                        .unwrap_or_else(|| "-".to_string()),
                    upload: entry.get("upload").and_then(Value::as_u64).unwrap_or(0),
                    download: entry.get("download").and_then(Value::as_u64).unwrap_or(0),
                    started: entry
                        .get("start_time")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    ctx: entry
                        .get("ctx")
                        .map(|ctx| ctx.to_string())
                        .unwrap_or_default(),
                });
            }
        }
        self.rows.sort_by(|a, b| b.download.cmp(&a.download));
    }

    // --- requests -------------------------------------------------------

    fn send(&mut self, method: &str, params: Value, kind: Pending) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.pending.insert(id, kind);
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        if let Some(conn) = &self.conn {
            conn.send(payload.to_string());
        }
        id
    }

    pub fn request_config(&mut self) {
        self.send("config.get", json!({}), Pending::Config);
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

fn parse_engine_status(value: &Value) -> EngineStatus {
    let state = match value.get("status") {
        Some(Value::String(state)) => state.clone(),
        Some(other) => other.to_string(),
        None => "Unknown".to_string(),
    };
    let mut servers = Vec::new();
    if let Some(list) = value.get("servers").and_then(Value::as_array) {
        for server in list {
            servers.push((
                server
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("-")
                    .to_string(),
                server
                    .get("server_type")
                    .and_then(Value::as_str)
                    .unwrap_or("-")
                    .to_string(),
                server
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("-")
                    .to_string(),
            ));
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
        assert_eq!(status.servers[0].0, "socks5");
        assert_eq!(status.servers[0].2, "Running");
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
}