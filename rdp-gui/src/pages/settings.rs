use gpui::{div, prelude::*, px, Context, Entity, IntoElement, Render, Window};

use crate::pages::{card, page_shell};
use crate::store::Store;
use crate::theme as t;

pub struct SettingsPage {
    store: Entity<Store>,
}

impl SettingsPage {
    pub fn new(store: Entity<Store>, _cx: &mut Context<Self>) -> Self {
        Self { store }
    }
}

impl Render for SettingsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let endpoint = store.endpoint.clone();
        let config_text = store
            .config
            .as_ref()
            .map(|config| serde_json::to_string_pretty(config).unwrap_or_default())
            .unwrap_or_else(|| "尚未取得配置".to_string());
        let preview: String = config_text.chars().take(8000).collect();

        page_shell(
            "设置",
            "引擎控制与当前配置",
            div()
                .flex()
                .flex_col()
                .gap_4()
                .child(card(
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(div().text_color(t::muted()).child("引擎"))
                        .child(div().text_xs().child(format!("API 端点：{endpoint}")))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .gap_2()
                                .child(
                                    div()
                                        .id("reload-config")
                                        .px_3()
                                        .py_1()
                                        .rounded_md()
                                        .border_1()
                                        .border_color(t::border())
                                        .cursor_pointer()
                                        .child("重新读取配置")
                                        .on_click(cx.listener(|this, _event, _window, cx| {
                                            this.store.update(cx, |store, cx| {
                                                store.request_config();
                                                cx.notify();
                                            });
                                        })),
                                )
                                .child(
                                    div()
                                        .id("stop-engine")
                                        .px_3()
                                        .py_1()
                                        .rounded_md()
                                        .border_1()
                                        .border_color(t::danger())
                                        .text_color(t::danger())
                                        .cursor_pointer()
                                        .child("停止引擎")
                                        .on_click(cx.listener(|this, _event, _window, cx| {
                                            this.store.update(cx, |store, _cx| {
                                                store.engine_stop();
                                            });
                                        })),
                                ),
                        ),
                ))
                .child(card(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(div().text_color(t::muted()).child("当前配置"))
                        .child(
                            div()
                                .h(px(360.))
                                .id("config-scroll")
                                .overflow_y_scroll()
                                .text_xs()
                                .child(preview),
                        ),
                )),
        )
    }
}
