//! Settings built the same way as the React page: subscriptions, the local
//! listener, TUN mode, and the engine controls. The form lives in userdata
//! (`daemon/settings`) and can be applied to the engine with one click.

use gpui::{
    div, prelude::*, px, AnyElement, Context, Entity, FontWeight, IntoElement, Render, Window,
};

use crate::components::text_field::TextField;
use crate::pages::{action, card, chip, page_shell};
use crate::store::{DaemonSettings, Notice, Store, Subscription};
use crate::theme as t;

pub struct SettingsPage {
    store: Entity<Store>,
    url_input: Entity<TextField>,
    port_input: Entity<TextField>,
    tun_ip_input: Entity<TextField>,
    mtu_input: Entity<TextField>,
    edit_input: Entity<TextField>,
    seeded: bool,
    editing: Option<usize>,
}

impl SettingsPage {
    pub fn new(store: Entity<Store>, cx: &mut Context<Self>) -> Self {
        let submit_store = store.clone();
        let url_input = cx.new(|cx| {
            TextField::new("订阅文件链接", "", cx).on_submit(move |text, _window, cx| {
                let text = text.trim().to_string();
                if text.is_empty() {
                    return;
                }
                submit_store.update(cx, |store, cx| {
                    store.add_subscription(text);
                    cx.notify();
                });
            })
        });
        let port_input = cx.new(|cx| TextField::new("10800", "", cx));
        let tun_ip_input = cx.new(|cx| TextField::new("10.0.0.1/24", "", cx));
        let mtu_input = cx.new(|cx| TextField::new("1400", "", cx));
        let edit_input = cx.new(|cx| TextField::new("订阅文件链接", "", cx));

        Self {
            store,
            url_input,
            port_input,
            tun_ip_input,
            mtu_input,
            edit_input,
            seeded: false,
            editing: None,
        }
    }

    fn seed(&mut self, settings: &DaemonSettings, cx: &mut Context<Self>) {
        self.seeded = true;
        let port = settings.port.to_string();
        let tun_ip = settings.tun_ip.clone();
        let mtu = settings.tun_mtu.to_string();
        self.port_input.update(cx, |field, _| field.set_text(port));
        self.tun_ip_input
            .update(cx, |field, _| field.set_text(tun_ip));
        self.mtu_input.update(cx, |field, _| field.set_text(mtu));
    }

    /// Reads the text fields into the form, rejecting values the engine would
    /// refuse. Returns `false` when a field is invalid.
    fn commit_fields(&mut self, cx: &mut Context<Self>) -> bool {
        // Before the stored settings arrive the fields are still empty; there is
        // nothing to read yet.
        if !self.seeded {
            return true;
        }
        let port_text = self.port_input.read(cx).text().trim().to_string();
        let tun_ip = self.tun_ip_input.read(cx).text().trim().to_string();
        let mtu_text = self.mtu_input.read(cx).text().trim().to_string();

        let port = match port_text.parse::<u16>() {
            Ok(port) if port > 0 => port,
            _ => {
                let store = self.store.clone();
                store.update(cx, |store, _| {
                    store.set_notice(false, "端口必须是 1-65535 之间的数字");
                });
                return false;
            }
        };
        let mtu = if mtu_text.is_empty() {
            None
        } else {
            match mtu_text.parse::<u16>() {
                Ok(mtu) if mtu > 0 => Some(mtu),
                _ => {
                    let store = self.store.clone();
                    store.update(cx, |store, _| {
                        store.set_notice(false, "MTU 必须是正整数");
                    });
                    return false;
                }
            }
        };

        self.store.update(cx, |store, _| {
            store.settings.port = port;
            store.settings.tun_ip = tun_ip;
            if let Some(mtu) = mtu {
                store.settings.tun_mtu = mtu;
            }
        });
        true
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if !self.commit_fields(cx) {
            return;
        }
        self.store.update(cx, |store, cx| {
            store.save_settings();
            cx.notify();
        });
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        if !self.commit_fields(cx) {
            return;
        }
        self.store.update(cx, |store, cx| {
            store.apply_settings();
            cx.notify();
        });
    }
}

fn subscription_row(
    store: &Entity<Store>,
    settings: &DaemonSettings,
    index: usize,
    sub: &Subscription,
    cx: &mut Context<SettingsPage>,
) -> AnyElement {
    let active = settings.active_index == index as i64;
    let use_store = store.clone();
    let refresh_store = store.clone();
    let remove_store = store.clone();
    let edit_url = sub.url.clone();

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_3()
        .py_2()
        .px_3()
        .rounded_md()
        .border_1()
        .border_color(if active { t::accent() } else { t::border() })
        .child(
            div()
                .w(px(240.))
                .min_w_0()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_ellipsis()
                        .child(sub.name.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(t::muted())
                        .text_ellipsis()
                        .child(sub.url.clone()),
                ),
        )
        .child(
            div()
                .w(px(110.))
                .text_xs()
                .text_color(t::muted())
                .child(format!("每 {} 分钟", sub.interval)),
        )
        .child(div().flex_1())
        .when(active, |el| el.child(chip(true).child("使用中")))
        .child(
            action()
                .id(("use-sub", index))
                .when(!active, |el| {
                    el.border_color(t::accent()).text_color(t::accent())
                })
                .child(if active { "重新应用" } else { "切换" })
                .on_click(cx.listener(move |this, _event, _window, cx| {
                    if !this.commit_fields(cx) {
                        return;
                    }
                    use_store.update(cx, |store, cx| {
                        store.activate_subscription(index);
                        cx.notify();
                    });
                })),
        )
        .child(
            action()
                .id(("edit-sub", index))
                .child("编辑")
                .on_click(cx.listener(move |this, _event, _window, cx| {
                    this.editing = Some(index);
                    this.edit_input
                        .update(cx, |field, _| field.set_text(edit_url.clone()));
                    cx.notify();
                })),
        )
        .child(
            action()
                .id(("refresh-sub", index))
                .child("刷新")
                .on_click(cx.listener(move |this, _event, _window, cx| {
                    if !this.commit_fields(cx) {
                        return;
                    }
                    refresh_store.update(cx, |store, cx| {
                        store.refresh_subscription(index);
                        cx.notify();
                    });
                })),
        )
        .child(
            action()
                .id(("remove-sub", index))
                .border_color(t::danger())
                .text_color(t::danger())
                .child("删除")
                .on_click(cx.listener(move |this, _event, _window, cx| {
                    if !this.commit_fields(cx) {
                        return;
                    }
                    remove_store.update(cx, |store, cx| {
                        store.remove_subscription(index);
                        cx.notify();
                    });
                })),
        )
        .into_any_element()
}

fn notice_card(notice: &Notice) -> AnyElement {
    card(
        div()
            .text_xs()
            .text_color(if notice.ok { t::ok() } else { t::danger() })
            .child(notice.text.clone()),
    )
    .into_any_element()
}

impl Render for SettingsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (settings, loaded, busy, notice, suggestion, engine_state, endpoint, config_text) = {
            let store = self.store.read(cx);
            let config_text = store
                .config
                .as_ref()
                .map(|config| serde_json::to_string_pretty(config).unwrap_or_default())
                .unwrap_or_else(|| "尚未取得配置".to_string());
            (
                store.settings.clone(),
                store.settings_loaded,
                store.settings_busy,
                store.settings_notice.clone(),
                store.tun_ip_suggestion.clone(),
                store.engine.state.clone(),
                store.endpoint.clone(),
                config_text,
            )
        };

        if loaded && !self.seeded {
            self.seed(&settings, cx);
        }

        let mut subscription_rows: Vec<AnyElement> = Vec::new();
        for (index, sub) in settings.subscriptions.iter().enumerate() {
            subscription_rows.push(subscription_row(&self.store, &settings, index, sub, cx));
        }

        let mut dns_chips: Vec<AnyElement> = Vec::new();
        for (index, (label, value)) in [("Cloudflare", "cloudflare"), ("Google", "google")]
            .into_iter()
            .enumerate()
        {
            let store = self.store.clone();
            dns_chips.push(
                chip(settings.bootstrap_dns == value)
                    .id(("dns", index))
                    .child(label)
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        if !this.commit_fields(cx) {
                            return;
                        }
                        store.update(cx, |store, cx| {
                            store.settings.bootstrap_dns = value.to_string();
                            store.save_settings();
                            cx.notify();
                        });
                    }))
                    .into_any_element(),
            );
        }

        let mut dns_mode_chips: Vec<AnyElement> = Vec::new();
        for (index, (label, value)) in [("Fake-IP", "fake-ip"), ("Redir-Host", "redir-host")]
            .into_iter()
            .enumerate()
        {
            let store = self.store.clone();
            dns_mode_chips.push(
                chip(settings.tun_dns_mode == value)
                    .id(("dns-mode", index))
                    .child(label)
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        if !this.commit_fields(cx) {
                            return;
                        }
                        store.update(cx, |store, cx| {
                            store.settings.tun_dns_mode = value.to_string();
                            store.save_settings();
                            cx.notify();
                        });
                    }))
                    .into_any_element(),
            );
        }

        let toggle_store = self.store.clone();
        let tun_toggle = chip(settings.tun_enabled)
            .id("tun-toggle")
            .child(if settings.tun_enabled {
                "已启用"
            } else {
                "已关闭"
            })
            .on_click(cx.listener(move |this, _event, _window, cx| {
                if !this.commit_fields(cx) {
                    return;
                }
                toggle_store.update(cx, |store, cx| {
                    store.settings.tun_enabled = !store.settings.tun_enabled;
                    store.save_settings();
                    cx.notify();
                });
            }));

        let suggest_store = self.store.clone();
        let suggestion_button = suggestion.clone().map(|ip| {
            action()
                .id("use-suggested-ip")
                .border_color(t::accent())
                .text_color(t::accent())
                .child(format!("使用 {ip}"))
                .on_click(cx.listener(move |this, _event, _window, cx| {
                    this.tun_ip_input
                        .update(cx, |field, _| field.set_text(ip.clone()));
                    suggest_store.update(cx, |store, _| {
                        store.settings.tun_ip = ip.clone();
                    });
                }))
        });

        let edit_card = self.editing.and_then(|index| {
            let sub = settings.subscriptions.get(index)?;
            let save_store = self.store.clone();
            Some(
                card(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .text_xs()
                                .text_color(t::muted())
                                .child(format!("编辑 {} 的订阅链接", sub.name)),
                        )
                        .child(div().w(px(420.)).child(self.edit_input.clone()))
                        .child(
                            action()
                                .id("save-edit")
                                .border_color(t::accent())
                                .text_color(t::accent())
                                .child("保存")
                                .on_click(cx.listener(move |this, _event, _window, cx| {
                                    if !this.commit_fields(cx) {
                                        return;
                                    }
                                    let Some(index) = this.editing else {
                                        return;
                                    };
                                    let url = this.edit_input.read(cx).text().trim().to_string();
                                    if url.is_empty() {
                                        return;
                                    }
                                    save_store.update(cx, |store, cx| {
                                        store.update_subscription_url(index, url);
                                        cx.notify();
                                    });
                                    this.editing = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            action()
                                .id("cancel-edit")
                                .child("取消")
                                .on_click(cx.listener(|this, _event, _window, cx| {
                                    this.editing = None;
                                    cx.notify();
                                })),
                        ),
                )
                .into_any_element(),
            )
        });

        let import_store = self.store.clone();
        let import_row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .child(div().w(px(420.)).child(self.url_input.clone()))
            .child(
                action()
                    .id("add-sub")
                    .border_color(t::accent())
                    .text_color(t::accent())
                    .child("导入")
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        if !this.commit_fields(cx) {
                            return;
                        }
                        let url = this.url_input.read(cx).text().trim().to_string();
                        if url.is_empty() {
                            return;
                        }
                        import_store.update(cx, |store, cx| {
                            store.add_subscription(url);
                            cx.notify();
                        });
                        this.url_input
                            .update(cx, |field, _| field.set_text(String::new()));
                    })),
            );

        let stop_store = self.store.clone();
        let reload_store = self.store.clone();

        page_shell(
            "设置",
            "订阅、代理端口、TUN 与引擎控制",
            div()
                .id("settings-scroll")
                .size_full()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_4()
                .when(!loaded, |el| {
                    el.child(card(
                        div()
                            .text_xs()
                            .text_color(t::muted())
                            .child("正在读取已保存的设置…"),
                    ))
                })
                .when(notice.is_some(), |el| {
                    el.child(notice_card(&notice.unwrap()))
                })
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
                                .gap_3()
                                .child(div().font_weight(FontWeight::SEMIBOLD).child("订阅管理"))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(t::muted())
                                        .child(format!("{} 个订阅", settings.subscriptions.len())),
                                ),
                        )
                        .child(import_row)
                        .when(subscription_rows.is_empty(), |el| {
                            el.child(
                                div()
                                    .text_xs()
                                    .text_color(t::muted())
                                    .child("还没有订阅，添加一个链接开始使用。"),
                            )
                        })
                        .children(subscription_rows)
                        .children(edit_card),
                ))
                .child(card(
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(div().font_weight(FontWeight::SEMIBOLD).child("代理设置"))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_3()
                                .child(
                                    div()
                                        .w(px(140.))
                                        .text_color(t::muted())
                                        .child("HTTP/SOCKS5 端口"),
                                )
                                .child(div().w(px(120.)).child(self.port_input.clone()))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(t::muted())
                                        .child(format!("监听 127.0.0.1:{}", settings.port)),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_3()
                                .child(div().w(px(140.)).text_color(t::muted()).child("节点 DNS"))
                                .child(div().flex().flex_row().gap_2().children(dns_chips)),
                        ),
                ))
                .child(card(
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(div().font_weight(FontWeight::SEMIBOLD).child("TUN 模式"))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_3()
                                .child(div().w(px(140.)).text_color(t::muted()).child("启用 TUN"))
                                .child(tun_toggle),
                        )
                        .when(settings.tun_enabled, |el| {
                            el.child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        div().w(px(140.)).text_color(t::muted()).child("DNS 模式"),
                                    )
                                    .child(
                                        div().flex().flex_row().gap_2().children(dns_mode_chips),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        div().w(px(140.)).text_color(t::muted()).child("IP 地址"),
                                    )
                                    .child(div().w(px(220.)).child(self.tun_ip_input.clone()))
                                    .children(suggestion_button),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_3()
                                    .child(div().w(px(140.)).text_color(t::muted()).child("MTU"))
                                    .child(div().w(px(120.)).child(self.mtu_input.clone())),
                            )
                        })
                        .child(
                            div()
                                .text_xs()
                                .text_color(t::muted())
                                .child(
                                    "Windows 下 TUN 需要以管理员身份启动，且 wintun.dll 必须与 rdp-gui.exe 同目录；目前仅接管 IPv4。",
                                ),
                        ),
                ))
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
                                .gap_3()
                                .child(div().font_weight(FontWeight::SEMIBOLD).child("引擎控制"))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(t::muted())
                                        .child(format!("{engine_state} · {endpoint}")),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .child(
                                    action()
                                        .id("apply-settings")
                                        .border_color(t::accent())
                                        .text_color(t::accent())
                                        .child(if busy {
                                            "处理中…"
                                        } else if engine_state == "Running" {
                                            "重新应用配置"
                                        } else {
                                            "启动"
                                        })
                                        .on_click(cx.listener(|this, _event, _window, cx| {
                                            this.apply(cx);
                                        })),
                                )
                                .child(action().id("save-settings").child("保存").on_click(
                                    cx.listener(|this, _event, _window, cx| {
                                        this.save(cx);
                                    }),
                                ))
                                .child(
                                    action()
                                        .id("stop-engine")
                                        .border_color(t::danger())
                                        .text_color(t::danger())
                                        .child("停止引擎")
                                        .on_click(cx.listener(
                                            move |_this, _event, _window, cx| {
                                                stop_store.update(cx, |store, cx| {
                                                    store.engine_stop();
                                                    cx.notify();
                                                });
                                            },
                                        )),
                                )
                                .child(
                                    action().id("reload-config").child("重新读取配置").on_click(
                                        cx.listener(move |_this, _event, _window, cx| {
                                            reload_store.update(cx, |store, cx| {
                                                store.request_config();
                                                cx.notify();
                                            });
                                        }),
                                    ),
                                ),
                        ),
                ))
                .child(card(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(div().text_color(t::muted()).child("引擎当前配置"))
                        .child(
                            div()
                                .h(px(280.))
                                .id("config-scroll")
                                .overflow_y_scroll()
                                .text_xs()
                                .child(config_text),
                        ),
                )),
        )
    }
}
