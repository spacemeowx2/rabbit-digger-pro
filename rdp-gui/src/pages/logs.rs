use gpui::{div, prelude::*, px, Context, Entity, IntoElement, Render, Window};

use crate::pages::{card, page_shell};
use crate::store::Store;
use crate::theme as t;

pub struct LogsPage {
    store: Entity<Store>,
}

impl LogsPage {
    pub fn new(store: Entity<Store>, _cx: &mut Context<Self>) -> Self {
        Self { store }
    }
}

fn level_color(level: &str) -> gpui::Rgba {
    match level {
        "ERROR" | "error" => t::danger(),
        "WARN" | "warn" => t::warn(),
        "INFO" | "info" => t::ok(),
        "DEBUG" | "debug" | "TRACE" | "trace" => t::muted(),
        _ => t::text(),
    }
}

impl Render for LogsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let lines: Vec<(String, String, String)> = store
            .logs
            .iter()
            .rev()
            .take(300)
            .map(|line| {
                (
                    line.level.clone(),
                    line.time.clone(),
                    line.message.clone(),
                )
            })
            .collect();
        let total = store.logs.len();

        let rows = lines.into_iter().map(|(level, time, message)| {
            let color = level_color(&level);
            div()
                .flex()
                .flex_row()
                .items_start()
                .gap_3()
                .py_1()
                .child(div().w(px(64.)).text_xs().text_color(color).child(level))
                .child(
                    div()
                        .w(px(96.))
                        .text_xs()
                        .text_color(t::muted())
                        .child(short_time(&time)),
                )
                .child(div().flex_1().child(message))
        });

        page_shell(
            "日志",
            "引擎日志流",
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .text_xs()
                                .text_color(t::muted())
                                .child(format!("最近 {total} 条")),
                        )
                        .child(
                            div()
                                .id("clear-logs")
                                .px_3()
                                .py_1()
                                .rounded_md()
                                .border_1()
                                .border_color(t::border())
                                .cursor_pointer()
                                .child("清空")
                                .on_click(cx.listener(|this, _event, _window, cx| {
                                    this.store.update(cx, |store, cx| {
                                        store.clear_logs();
                                        cx.notify();
                                    });
                                })),
                        ),
                )
                .child(card(
                    div()
                        .flex()
                        .flex_col()
                        .h(px(480.))
                        .id("logs-scroll")
                        .overflow_y_scroll()
                        .children(rows),
                )),
        )
    }
}

fn short_time(time: &str) -> String {
    // Timestamps are RFC3339; the clock part is enough for a log list.
    time.get(11..19).unwrap_or(time).to_string()
}
