//! Application shell: a fixed sidebar and a page area that swaps between views.

use gpui::{
    div, prelude::*, px, Context, Entity, FontWeight, IntoElement, Render, Window,
};

use crate::pages;
use crate::store::Store;
use crate::theme as t;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Status,
    Connections,
    Logs,
    SelectNets,
    Settings,
}

impl Page {
    pub const ALL: [Page; 5] = [
        Page::Status,
        Page::Connections,
        Page::Logs,
        Page::SelectNets,
        Page::Settings,
    ];

    fn label(self) -> &'static str {
        match self {
            Page::Status => "状态",
            Page::Connections => "连接",
            Page::Logs => "日志",
            Page::SelectNets => "节点",
            Page::Settings => "设置",
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Page::Status => "◉",
            Page::Connections => "⇄",
            Page::Logs => "≡",
            Page::SelectNets => "◇",
            Page::Settings => "⚙",
        }
    }
}

pub struct Root {
    page: Page,
    store: Entity<Store>,
    status: Entity<pages::status::StatusPage>,
    connections: Entity<pages::connections::ConnectionsPage>,
    logs: Entity<pages::logs::LogsPage>,
    select_nets: Entity<pages::select_nets::SelectNetsPage>,
    settings: Entity<pages::settings::SettingsPage>,
}

impl Root {
    pub fn new(endpoint: String, cx: &mut Context<Self>) -> Self {
        let store = cx.new(|cx| Store::new(endpoint, cx));
        let status_store = store.clone();
        let connections_store = store.clone();
        let logs_store = store.clone();
        let nets_store = store.clone();
        let settings_store = store.clone();

        Self {
            page: Page::Status,
            store: store.clone(),
            status: cx.new(|cx| pages::status::StatusPage::new(status_store, cx)),
            connections: cx.new(|cx| pages::connections::ConnectionsPage::new(connections_store, cx)),
            logs: cx.new(|cx| pages::logs::LogsPage::new(logs_store, cx)),
            select_nets: cx.new(|cx| pages::select_nets::SelectNetsPage::new(nets_store, cx)),
            settings: cx.new(|cx| pages::settings::SettingsPage::new(settings_store, cx)),
        }
    }

    fn sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.page;
        let connected = self.store.read(cx).connected;
        let state = self.store.read(cx).engine.state.clone();

        div()
            .w(px(212.))
            .h_full()
            .flex()
            .flex_col()
            .bg(t::sidebar_bg())
            .border_r_1()
            .border_color(t::border())
            .p_3()
            .gap_1()
            .child(
                div()
                    .px_3()
                    .pt_3()
                    .pb_5()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Rabbit Digger"),
                    )
                    .child(div().text_xs().text_color(t::muted()).child(format!(
                        "{state} · {}",
                        if connected { "已连接" } else { "未连接" }
                    ))),
            )
            .children(Page::ALL.into_iter().enumerate().map(|(index, page)| {
                let active = page == current;
                div()
                    .id(("nav", index))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .cursor_pointer()
                    .when(active, |el| el.bg(t::accent_soft()).text_color(t::accent()))
                    .when(!active, |el| el.text_color(t::muted()))
                    .child(div().w(px(16.)).child(page.glyph()))
                    .child(div().child(page.label()))
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        if this.page != page {
                            this.page = page;
                            cx.notify();
                        }
                    }))
            }))
            .child(div().flex_1())
            .child(
                div()
                    .px_3()
                    .pb_2()
                    .text_xs()
                    .text_color(t::muted())
                    .child("GPUI · native + web"),
            )
    }

    fn content(&mut self) -> impl IntoElement {
        match self.page {
            Page::Status => div().size_full().child(self.status.clone()),
            Page::Connections => div().size_full().child(self.connections.clone()),
            Page::Logs => div().size_full().child(self.logs.clone()),
            Page::SelectNets => div().size_full().child(self.select_nets.clone()),
            Page::Settings => div().size_full().child(self.settings.clone()),
        }
    }
}

impl Render for Root {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = self.content();
        let sidebar = self.sidebar(cx);

        div()
            .size_full()
            .flex()
            .flex_row()
            .bg(t::window_bg())
            .text_color(t::text())
            .text_sm()
            .child(sidebar)
            .child(div().flex_1().h_full().overflow_hidden().child(body))
    }
}
