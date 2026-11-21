use gpui::{
    div, prelude::*, px, Context, Entity, FontWeight, IntoElement, Render, Window,
};

use crate::pages::{card, fmt_bytes, page_shell};
use crate::store::Store;
use crate::theme as t;

pub struct ConnectionsPage {
    store: Entity<Store>,
}

struct Row {
    id: String,
    protocol: String,
    addr: String,
    upload: u64,
    download: u64,
    ctx: String,
}

impl ConnectionsPage {
    pub fn new(store: Entity<Store>, _cx: &mut Context<Self>) -> Self {
        Self { store }
    }
}

impl Render for ConnectionsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<Row> = self
            .store
            .read(cx)
            .rows
            .iter()
            .map(|row| Row {
                id: row.id.clone(),
                protocol: row.protocol.clone(),
                addr: row.addr.clone(),
                upload: row.upload,
                download: row.download,
                ctx: row.ctx.clone(),
            })
            .collect();
        let count = rows.len();
        let store = self.store.clone();

        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_4()
            .px_3()
            .py_2()
            .text_xs()
            .text_color(t::muted())
            .child(div().w(px(70.)).child("协议"))
            .child(div().flex_1().child("目标"))
            .child(div().w(px(96.)).child("上行"))
            .child(div().w(px(96.)).child("下行"))
            .child(div().w(px(64.)).child("操作"));

        let body = rows.into_iter().enumerate().map(|(index, row)| {
            let store = store.clone();
            let id = row.id.clone();
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_4()
                .px_3()
                .py_2()
                .rounded_md()
                .border_b_1()
                .border_color(t::border())
                .child(
                    div()
                        .w(px(70.))
                        .text_color(t::accent())
                        .child(row.protocol),
                )
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().child(row.addr))
                        .when(!row.ctx.is_empty(), |el| {
                            el.child(
                                div()
                                    .text_xs()
                                    .text_color(t::muted())
                                    .child(truncate(&row.ctx, 90)),
                            )
                        }),
                )
                .child(div().w(px(96.)).child(fmt_bytes(row.upload)))
                .child(div().w(px(96.)).child(fmt_bytes(row.download)))
                .child(
                    div()
                        .id(("close", index))
                        .w(px(64.))
                        .cursor_pointer()
                        .text_color(t::danger())
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
                        .child(
                            div()
                                .id("close-all")
                                .px_3()
                                .py_1()
                                .rounded_md()
                                .border_1()
                                .border_color(t::danger())
                                .text_color(t::danger())
                                .cursor_pointer()
                                .child("全部断开")
                                .on_click(cx.listener(|this, _event, _window, cx| {
                                    this.store.update(cx, |store, _cx| {
                                        store.close_all_connections();
                                    });
                                })),
                        ),
                )
                .child(
                    card(div()
                        .flex()
                        .flex_col()
                        .child(header)
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
                        })),
                ),
        )
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
