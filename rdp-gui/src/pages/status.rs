use gpui::{div, prelude::*, px, Context, Entity, FontWeight, IntoElement, Render, Window};

use crate::pages::{card, fmt_bytes, fmt_rate, metric, page_shell, status_pill};
use crate::store::{ServerRow, Store, TRAFFIC_POINTS};
use crate::theme as t;

pub struct StatusPage {
    store: Entity<Store>,
}

impl StatusPage {
    pub fn new(store: Entity<Store>, _cx: &mut Context<Self>) -> Self {
        Self { store }
    }

    fn sparkline(samples: &[(f64, f64)]) -> impl IntoElement {
        let peak = samples
            .iter()
            .map(|(up, down)| up.max(*down))
            .fold(0.0_f64, f64::max)
            .max(1.0);

        let bars = samples.iter().map(move |(up, down)| {
            let up_height = ((up / peak) * 120.0).max(1.0) as f32;
            let down_height = ((down / peak) * 120.0).max(1.0) as f32;
            div()
                .flex()
                .flex_row()
                .items_end()
                .gap_1()
                .child(div().w(px(3.)).h(px(up_height)).bg(t::accent()))
                .child(div().w(px(3.)).h(px(down_height)).bg(t::ok()))
        });

        div()
            .flex()
            .flex_row()
            .items_end()
            .gap_1()
            .h(px(124.))
            .children(bars)
    }
}

fn state_label(state: &str) -> &'static str {
    match state {
        "Idle" => "空闲",
        "Starting" => "启动中",
        "Running" => "运行中",
        "Stopping" => "停止中",
        "Error" => "错误",
        "Connecting" => "连接中",
        _ => "未知",
    }
}

fn server_row(server: &ServerRow) -> impl IntoElement {
    let color = match server.status.as_str() {
        "Running" => t::ok(),
        "Error" => t::danger(),
        _ => t::muted(),
    };

    div()
        .flex()
        .flex_row()
        .items_start()
        .gap_3()
        .py_2()
        .border_b_1()
        .border_color(t::border())
        .child(div().mt_2().w(px(8.)).h(px(8.)).rounded_full().bg(color))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_sm().text_ellipsis().child(server.name.clone()))
                .when(server.message.is_some(), |el| {
                    el.child(
                        div()
                            .text_xs()
                            .text_color(t::danger())
                            .child(server.message.clone().unwrap_or_default()),
                    )
                }),
        )
        .child(
            div()
                .text_xs()
                .text_color(t::muted())
                .child(server.kind.clone()),
        )
        .child(
            div()
                .text_xs()
                .text_color(color)
                .child(server.status.clone()),
        )
}

impl Render for StatusPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let state = store.engine.state.clone();
        let engine_message = store.engine.message.clone();
        let servers = store.engine.servers.clone();
        let up_rate = store.up_rate;
        let down_rate = store.down_rate;
        let rows = store.rows.len();
        let total_up = store.total_up;
        let total_down = store.total_down;
        let error = store.error.clone();
        let connected = store.connected;

        let running_servers = servers
            .iter()
            .filter(|server| server.status == "Running")
            .count();

        let mut samples: Vec<(f64, f64)> = store.traffic.iter().copied().collect();
        while samples.len() < TRAFFIC_POINTS {
            samples.insert(0, (0.0, 0.0));
        }

        page_shell(
            "状态",
            "引擎运行状态与实时流量",
            div()
                .id("status-scroll")
                .size_full()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_4()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .child(status_pill(&state))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(state_label(&state)),
                        )
                        .child(div().text_xs().text_color(t::muted()).child(format!(
                            "连接数 {rows} · 会话上行 {} · 下行 {}",
                            fmt_bytes(total_up),
                            fmt_bytes(total_down)
                        )))
                        .when(!connected, |el| {
                            el.child(
                                div()
                                    .text_xs()
                                    .text_color(t::danger())
                                    .child("未连接到引擎"),
                            )
                        }),
                )
                .when(error.is_some(), |el| {
                    el.child(card(
                        div()
                            .text_color(t::danger())
                            .child(error.unwrap_or_default()),
                    ))
                })
                .when(engine_message.is_some(), |el| {
                    el.child(card(
                        div()
                            .text_xs()
                            .text_color(t::warn())
                            .child(engine_message.clone().unwrap_or_default()),
                    ))
                })
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_4()
                        .child(
                            div()
                                .flex_1()
                                .child(metric("上行速率", fmt_rate(up_rate), true)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .child(metric("下行速率", fmt_rate(down_rate), false)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .child(metric("活跃连接", rows.to_string(), false)),
                        )
                        .child(div().flex_1().child(metric(
                            "累计流量",
                            fmt_bytes(total_up + total_down),
                            false,
                        ))),
                )
                .child(card(
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_4()
                                .child(div().text_color(t::muted()).child("流量曲线"))
                                .child(div().text_xs().text_color(t::accent()).child("上行"))
                                .child(div().text_xs().text_color(t::ok()).child("下行")),
                        )
                        .child(Self::sparkline(&samples)),
                ))
                .child(card(
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .pb_2()
                                .child(div().font_weight(FontWeight::SEMIBOLD).child("Servers"))
                                .child(
                                    div().text_xs().text_color(t::muted()).child(format!(
                                        "{running_servers}/{} running",
                                        servers.len()
                                    )),
                                ),
                        )
                        .when(servers.is_empty(), |el| {
                            el.child(div().py_6().text_xs().text_color(t::muted()).child(
                                if state == "Idle" {
                                    "引擎未启动"
                                } else {
                                    "暂无 Server 信息"
                                },
                            ))
                        })
                        .children(servers.iter().map(server_row)),
                )),
        )
    }
}
