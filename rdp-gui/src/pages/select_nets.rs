use gpui::{
    div, prelude::*, px, AnyElement, Context, Entity, FontWeight, IntoElement, Render, Window,
};

use crate::pages::{card, page_shell};
use crate::store::Store;
use crate::theme as t;

pub struct SelectNetsPage {
    store: Entity<Store>,
}

struct NetRow {
    name: String,
    kind: String,
    selected: Option<String>,
    candidates: Vec<String>,
}

impl SelectNetsPage {
    pub fn new(store: Entity<Store>, _cx: &mut Context<Self>) -> Self {
        Self { store }
    }
}

fn collect_nets(config: &serde_json::Value) -> Vec<NetRow> {
    let Some(nets) = config.get("net").and_then(|net| net.as_object()) else {
        return Vec::new();
    };

    nets.iter()
        .map(|(name, value)| {
            let selected = value
                .get("selected")
                .and_then(|selected| selected.as_str())
                .map(str::to_string);
            let candidates = value
                .get("list")
                .and_then(|list| list.as_array())
                .map(|list| {
                    list.iter()
                        .filter_map(|item| item.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            NetRow {
                name: name.clone(),
                kind: value
                    .get("type")
                    .and_then(|kind| kind.as_str())
                    .unwrap_or("-")
                    .to_string(),
                selected,
                candidates,
            }
        })
        .collect()
}

impl Render for SelectNetsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let nets = match self.store.read(cx).config.as_ref() {
            Some(config) => collect_nets(config),
            None => Vec::new(),
        };

        // Build the cards eagerly: `cx.listener` borrows the context, which a
        // nested `map` closure cannot hold while it iterates.
        let mut cards: Vec<AnyElement> = Vec::with_capacity(nets.len());
        for (net_index, net) in nets.into_iter().enumerate() {
            let mut chips: Vec<AnyElement> = Vec::with_capacity(net.candidates.len());
            for (chip_index, candidate) in net.candidates.iter().enumerate() {
                let store = self.store.clone();
                let net_name = net.name.clone();
                let candidate_name = candidate.clone();
                let active = net.selected.as_deref() == Some(candidate.as_str());

                chips.push(
                    div()
                        .id(("chip", net_index * 100 + chip_index))
                        .px_3()
                        .py_1()
                        .rounded_full()
                        .border_1()
                        .border_color(if active { t::accent() } else { t::border() })
                        .when(active, |el| el.bg(t::accent_soft()).text_color(t::accent()))
                        .when(!active, |el| el.text_color(t::muted()))
                        .cursor_pointer()
                        .child(candidate.clone())
                        .on_click(cx.listener(move |_this, _event, _window, cx| {
                            store.update(cx, |store, _cx| {
                                store.select_net(&net_name, &candidate_name);
                            });
                        }))
                        .into_any_element(),
                );
            }

            let selected_label = net
                .selected
                .clone()
                .unwrap_or_else(|| "未选择".to_string());

            let mut body = div()
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
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(net.name.clone()),
                        )
                        .child(div().text_xs().text_color(t::muted()).child(net.kind))
                        .child(
                            div()
                                .text_xs()
                                .text_color(t::accent())
                                .child(format!("当前：{selected_label}")),
                        ),
                );

            if chips.is_empty() {
                body = body.child(
                    div()
                        .text_xs()
                        .text_color(t::muted())
                        .child("这个 net 没有可切换的候选"),
                );
            } else {
                body = body.child(div().flex().flex_row().flex_wrap().gap_2().children(chips));
            }

            cards.push(card(body).into_any_element());
        }

        page_shell(
            "节点",
            "可切换的 net 与候选节点",
            div().flex().flex_col().gap_3().children(cards).child(
                div()
                    .text_xs()
                    .text_color(t::muted())
                    .child("点击候选即可切换，切换结果会从引擎推回。"),
            ),
        )
    }
}
