use gpui::{div, prelude::*, px, AnyElement, Context, Entity, IntoElement, Render, Window};

use crate::components::text_field::TextField;
use crate::pages::{action, card, chip, page_shell};
use crate::store::{LogLine, Store};
use crate::theme as t;

pub struct LogsPage {
    store: Entity<Store>,
    filter_input: Entity<TextField>,
    level: Option<String>,
    paused: bool,
    frozen: Vec<LogLine>,
}

impl LogsPage {
    pub fn new(store: Entity<Store>, cx: &mut Context<Self>) -> Self {
        let filter_input = cx.new(|cx| TextField::new("过滤日志…", "", cx));
        Self {
            store,
            filter_input,
            level: None,
            paused: false,
            frozen: Vec::new(),
        }
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
        let logs: Vec<LogLine> = self.store.read(cx).logs.iter().cloned().collect();
        let total = logs.len();
        let visible = if self.paused {
            self.frozen.clone()
        } else {
            logs.clone()
        };

        let mut levels: Vec<String> = Vec::new();
        for line in &visible {
            if !levels.contains(&line.level) {
                levels.push(line.level.clone());
            }
        }
        levels.sort();

        let query = self.filter_input.read(cx).text().trim().to_lowercase();
        let filtered: Vec<LogLine> = visible
            .iter()
            .rev()
            .filter(|line| {
                if let Some(level) = &self.level {
                    if &line.level != level {
                        return false;
                    }
                }
                if query.is_empty() {
                    return true;
                }
                format!("{} {} {}", line.time, line.level, line.message)
                    .to_lowercase()
                    .contains(&query)
            })
            .take(300)
            .cloned()
            .collect();
        let shown = filtered.len();

        let mut level_chips: Vec<AnyElement> = Vec::new();
        {
            let all_active = self.level.is_none();
            level_chips.push(
                chip(all_active)
                    .id("level-all")
                    .child("全部")
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        this.level = None;
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        for (index, level) in levels.iter().enumerate() {
            let level = level.clone();
            let active = self.level.as_deref() == Some(level.as_str());
            level_chips.push(
                chip(active)
                    .id(("level", index))
                    .child(level.clone())
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        this.level = Some(level.clone());
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }

        let rows = filtered.into_iter().map(|line| {
            let color = level_color(&line.level);
            div()
                .flex()
                .flex_row()
                .items_start()
                .gap_3()
                .py_1()
                .child(
                    div()
                        .w(px(56.))
                        .text_xs()
                        .text_color(color)
                        .child(line.level),
                )
                .child(
                    div()
                        .w(px(96.))
                        .text_xs()
                        .text_color(t::muted())
                        .child(short_time(&line.time)),
                )
                .child(div().flex_1().child(line.message))
        });

        let pause_label = if self.paused { "继续" } else { "暂停" };
        let pause_store = self.store.clone();

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
                        .gap_2()
                        .flex_wrap()
                        .child(
                            div()
                                .text_xs()
                                .text_color(t::muted())
                                .child(format!("{shown} / {total} 条")),
                        )
                        .children(level_chips)
                        .child(div().w(px(220.)).child(self.filter_input.clone()))
                        .child(
                            action()
                                .id("pause-logs")
                                .child(pause_label)
                                .on_click(cx.listener(move |this, _event, _window, cx| {
                                    if !this.paused {
                                        this.frozen =
                                            pause_store.read(cx).logs.iter().cloned().collect();
                                        this.paused = true;
                                    } else {
                                        this.paused = false;
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(
                            action()
                                .id("clear-logs")
                                .border_color(t::danger())
                                .text_color(t::danger())
                                .child("清空")
                                .on_click(cx.listener(|this, _event, _window, cx| {
                                    this.frozen.clear();
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
                        .h(px(520.))
                        .id("logs-scroll")
                        .overflow_y_scroll()
                        .children(rows)
                        .when(shown == 0, |el| {
                            el.child(
                                div()
                                    .py_8()
                                    .text_xs()
                                    .text_color(t::muted())
                                    .child("还没有新的日志"),
                            )
                        }),
                )),
        )
    }
}

fn short_time(time: &str) -> String {
    // Timestamps are RFC3339; the clock part is enough for a log list.
    time.get(11..19).unwrap_or(time).to_string()
}
