use gpui::{div, prelude::*, px, Context, Entity, FontWeight, IntoElement, Render, Window};

use crate::components::text_field::TextField;
use crate::pages::{action, card, fmt_bytes, page_shell};
use crate::store::{now_secs, ConnRow, Store};
use crate::theme as t;

pub struct ConnectionsPage {
    store: Entity<Store>,
    filter_input: Entity<TextField>,
}

impl ConnectionsPage {
    pub fn new(store: Entity<Store>, cx: &mut Context<Self>) -> Self {
        let filter_input = cx.new(|cx| TextField::new("过滤域名、链路…", "", cx));
        Self {
            store,
            filter_input,
        }
    }
}

fn format_age(started: u64) -> String {
    if started == 0 {
        return "-".to_string();
    }
    let seconds = now_secs().saturating_sub(started);
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m", seconds / 60)
    } else if seconds < 86400 {
        format!("{}h", seconds / 3600)
    } else {
        format!("{}d", seconds / 86400)
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let mut out: String = text.chars().take(max).collect();
        out.push('…');
        out
    }
}

impl Render for ConnectionsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.filter_input.read(cx).text().trim().to_lowercase();
        let rows: Vec<ConnRow> = {
            let store = self.store.read(cx);
            store
                .rows
                .iter()
                .filter(|row| {
                    if query.is_empty() {
                        return true;
                    }
                    format!("{} {} {} {}", row.host, row.route, row.source, row.protocol)
                        .to_lowercase()
                        .contains(&query)
                })
                .cloned()
                .collect()
        };
        let count = rows.len();
        let store = self.store.clone();

        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .px_3()
            .py_2()
            .text_xs()
            .text_color(t::muted())
            .child(div().flex_1().child("主机"))
            .child(div().w(px(56.)).child("协议"))
            .child(div().w(px(56.)).child("时长"))
            .child(div().w(px(88.)).child("上行"))
            .child(div().w(px(88.)).child("下行"))
            .child(div().w(px(180.)).child("链路"))
            .child(div().w(px(48.)).child("操作"));

        let body = rows.into_iter().enumerate().map(|(index, row)| {
            let store = store.clone();
            let id = row.id.clone();
            let host = truncate(&row.host, 80);
            let source = truncate(&row.source, 64);
            let route = truncate(&row.route, 48);
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_3()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(t::border())
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_ellipsis()
                                .child(host),
                        )
                        .when(!source.is_empty(), |el| {
                            el.child(
                                div()
                                    .text_xs()
                                    .text_color(t::muted())
                                    .text_ellipsis()
                                    .child(source),
                            )
                        }),
                )
                .child(
                    div()
                        .w(px(56.))
                        .text_xs()
                        .text_color(t::accent())
                        .child(row.protocol.to_uppercase()),
                )
                .child(
                    div()
                        .w(px(56.))
                        .text_xs()
                        .text_color(t::muted())
                        .child(format_age(row.started)),
                )
                .child(div().w(px(88.)).text_xs().child(fmt_bytes(row.upload)))
                .child(div().w(px(88.)).text_xs().child(fmt_bytes(row.download)))
                .child(
                    div()
                        .w(px(180.))
                        .min_w_0()
                        .text_xs()
                        .text_color(t::muted())
                        .text_ellipsis()
                        .child(route),
                )
                .child(
                    div()
                        .id(("close", index))
                        .w(px(48.))
                        .text_xs()
                        .text_color(t::danger())
                        .cursor_pointer()
                        .child("断开")
                        .on_click(cx.listener(move |_this, _event, _window, cx| {
                            store.update(cx, |store, _cx| {
                                store.close_connection(&id);
                            });
                        })),
                )
        });

        page_shell(
            "连接",
            "当前活跃的代理连接",
            div()
                .size_full()
                .min_h_0()
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
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(format!("{count} 条连接")),
                        )
                        .child(div().w(px(300.)).child(self.filter_input.clone()))
                        .child(div().flex_1())
                        .child(
                            action()
                                .id("close-all")
                                .border_color(t::danger())
                                .text_color(t::danger())
                                .child("全部断开")
                                .on_click(cx.listener(|this, _event, _window, cx| {
                                    this.store.update(cx, |store, _cx| {
                                        store.close_all_connections();
                                    });
                                })),
                        ),
                )
                .child(
                    card(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h_0()
                            .child(header)
                            .child(
                                div()
                                    .id("connections-scroll")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .children(body)
                                    .when(count == 0, |el| {
                                        el.child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .h(px(120.))
                                                .text_color(t::muted())
                                                .child("没有活跃连接"),
                                        )
                                    }),
                            ),
                    )
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0(),
                ),
        )
    }
}
