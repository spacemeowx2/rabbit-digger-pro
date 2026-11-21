use gpui::{
    div, prelude::*, px, AnyElement, Context, Entity, FontWeight, IntoElement, Render, Window,
};

use crate::components::text_field::TextField;
use crate::pages::{action, card, chip, page_shell};
use crate::store::{DelayState, Store};
use crate::theme as t;

pub struct SelectNetsPage {
    store: Entity<Store>,
    search_input: Entity<TextField>,
}

struct NetRow {
    name: String,
    kind: String,
    selected: Option<String>,
    candidates: Vec<String>,
    auto: bool,
    interval: Option<u64>,
    tolerance: Option<u64>,
}

impl SelectNetsPage {
    pub fn new(store: Entity<Store>, cx: &mut Context<Self>) -> Self {
        let search_input = cx.new(|cx| TextField::new("搜索节点…", "", cx));
        Self {
            store,
            search_input,
        }
    }
}

fn collect_nets(config: &serde_json::Value) -> Vec<NetRow> {
    let Some(nets) = config.get("net").and_then(|net| net.as_object()) else {
        return Vec::new();
    };

    nets.iter()
        .filter_map(|(name, value)| {
            let kind = value
                .get("type")
                .and_then(|kind| kind.as_str())
                .unwrap_or("-")
                .to_string();
            if !matches!(kind.as_str(), "select" | "url-test" | "fallback") {
                return None;
            }
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
            Some(NetRow {
                name: name.clone(),
                auto: matches!(kind.as_str(), "url-test" | "fallback"),
                kind,
                selected,
                candidates,
                interval: value.get("interval").and_then(|value| value.as_u64()),
                tolerance: value.get("tolerance").and_then(|value| value.as_u64()),
            })
        })
        .collect()
}

fn delay_label(state: Option<&DelayState>) -> Option<(String, gpui::Rgba)> {
    match state {
        None => None,
        Some(DelayState::Testing) => Some(("测速中…".to_string(), t::muted())),
        Some(DelayState::Done { response, .. }) => {
            let color = if *response <= 200 {
                t::ok()
            } else if *response <= 500 {
                t::warn()
            } else {
                t::danger()
            };
            Some((format!("{response}ms"), color))
        }
        Some(DelayState::Timeout) => Some(("超时".to_string(), t::danger())),
        Some(DelayState::Error) => Some(("错误".to_string(), t::danger())),
    }
}

impl Render for SelectNetsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search_input.read(cx).text().trim().to_lowercase();
        let (config, delays, scope) = {
            let store = self.store.read(cx);
            (
                store.config.clone(),
                store.delays.clone(),
                store.delay_scope.clone(),
            )
        };
        let mut nets = match config.as_ref() {
            Some(config) => collect_nets(config),
            None => Vec::new(),
        };
        if !query.is_empty() {
            nets = nets
                .into_iter()
                .filter_map(|mut net| {
                    if net.name.to_lowercase().contains(&query) {
                        return Some(net);
                    }
                    net.candidates
                        .retain(|candidate| candidate.to_lowercase().contains(&query));
                    if net.candidates.is_empty() {
                        None
                    } else {
                        Some(net)
                    }
                })
                .collect();
        }

        // Build the cards eagerly: `cx.listener` borrows the context, which a
        // nested `map` closure cannot hold while it iterates.
        let mut cards: Vec<AnyElement> = Vec::with_capacity(nets.len());
        let mut all_targets: Vec<(String, String)> = Vec::new();
        for (net_index, net) in nets.into_iter().enumerate() {
            let mut chips: Vec<AnyElement> = Vec::with_capacity(net.candidates.len());
            for (chip_index, candidate) in net.candidates.iter().enumerate() {
                let store = self.store.clone();
                let net_name = net.name.clone();
                let candidate_name = candidate.clone();
                let active = net.selected.as_deref() == Some(candidate.as_str());
                let delay = delay_label(delays.get(candidate));

                let label = div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(candidate.clone())
                    .when(delay.is_some(), |el| {
                        let (label, color) = delay.clone().unwrap();
                        el.child(div().text_xs().text_color(color).child(label))
                    });

                let candidate_chip: AnyElement = if net.auto {
                    // Auto groups pick at runtime; their cards are read-only.
                    chip(false).cursor_default().child(label).into_any_element()
                } else {
                    chip(active)
                        .id(("chip", net_index * 1000 + chip_index))
                        .child(label)
                        .on_click(cx.listener(move |_this, _event, _window, cx| {
                            store.update(cx, |store, _cx| {
                                store.select_net(&net_name, &candidate_name);
                            });
                        }))
                        .into_any_element()
                };
                chips.push(candidate_chip);
                all_targets.push((candidate.clone(), candidate.clone()));
            }

            let selected_label = net.selected.clone().unwrap_or_else(|| "未选择".to_string());
            let group_targets: Vec<(String, String)> = net
                .candidates
                .iter()
                .map(|candidate| (candidate.clone(), candidate.clone()))
                .collect();
            let test_store = self.store.clone();
            let test_name = net.name.clone();
            let testing_scope = scope.clone();
            let kind_label = match net.kind.as_str() {
                "url-test" => "URL 测速",
                "fallback" => "故障转移",
                _ => "手动选择",
            };
            let meta = if net.auto {
                let mut facts = Vec::new();
                if let Some(interval) = net.interval {
                    facts.push(format!("每 {interval}s 自动检测"));
                }
                if net.kind == "url-test" {
                    if let Some(tolerance) = net.tolerance {
                        facts.push(format!("容差 {tolerance}ms"));
                    }
                }
                facts.join(" · ")
            } else {
                format!("当前：{selected_label}")
            };

            let mut body = div().flex().flex_col().gap_3().child(
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
                    .child(div().text_xs().text_color(t::muted()).child(kind_label))
                    .child(
                        div()
                            .text_xs()
                            .text_color(if net.auto { t::ok() } else { t::accent() })
                            .child(meta),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_xs()
                            .text_color(t::muted())
                            .child(format!("{} 个候选", net.candidates.len())),
                    )
                    .child(
                        action()
                            .id(("test-group", net_index))
                            .when(testing_scope.is_some(), |el| el.opacity(0.5))
                            .child(if testing_scope.as_deref() == Some(test_name.as_str()) {
                                "测速中…"
                            } else {
                                "测速本组"
                            })
                            .on_click(cx.listener(move |_this, _event, _window, cx| {
                                let targets = group_targets.clone();
                                let scope = test_name.clone();
                                test_store.update(cx, |store, cx| {
                                    store.test_delays(&scope, targets);
                                    cx.notify();
                                });
                            })),
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
                if net.auto {
                    body = body.child(
                        div()
                            .text_xs()
                            .text_color(t::muted())
                            .child("自动组会在引擎内测速后切换，不能在这里手动固定。"),
                    );
                }
            }

            cards.push(card(body).into_any_element());
        }

        let test_store = self.store.clone();
        let testing = scope.is_some();
        page_shell(
            "节点",
            "可切换的 net 与候选节点",
            div()
                .id("nets-scroll")
                .size_full()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(div().w(px(300.)).child(self.search_input.clone()))
                        .child(
                            action()
                                .id("test-all")
                                .when(testing, |el| el.opacity(0.5))
                                .child(if testing {
                                    "测速中…"
                                } else {
                                    "测速全部"
                                })
                                .on_click(cx.listener(move |_this, _event, _window, cx| {
                                    let targets = all_targets.clone();
                                    test_store.update(cx, |store, cx| {
                                        store.test_delays("全部", targets);
                                        cx.notify();
                                    });
                                })),
                        ),
                )
                .children(cards)
                .child(
                    div()
                        .text_xs()
                        .text_color(t::muted())
                        .child("手动组点击候选即可切换；URL 测速/故障转移组由引擎自动选择。"),
                ),
        )
    }
}
