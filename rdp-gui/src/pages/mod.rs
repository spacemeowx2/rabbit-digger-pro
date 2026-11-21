pub mod connections;
pub mod logs;
pub mod select_nets;
pub mod settings;
pub mod status;

use gpui::{div, prelude::*, px, FontWeight, IntoElement, ParentElement, Styled};

use crate::theme as t;

/// Shared page chrome: title, one-line summary, and the area below it.
pub fn page_shell(
    title: &'static str,
    subtitle: &'static str,
    body: impl IntoElement,
) -> impl IntoElement {
    div()
        .size_full()
        .flex()
        .flex_col()
        .p_6()
        .gap_5()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xl()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(div().text_xs().text_color(t::muted()).child(subtitle)),
        )
        .child(div().flex_1().min_h_0().child(body))
}

/// A bordered panel used by every page.
pub fn card(body: impl IntoElement) -> impl IntoElement {
    div()
        .rounded_lg()
        .border_1()
        .border_color(t::border())
        .bg(t::card_bg())
        .p_4()
        .child(body)
}

/// Placeholder shown until a page has live data.
pub fn placeholder(text: &'static str) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_center()
        .h(px(160.))
        .text_color(t::muted())
        .child(text)
}

/// One headline number with its label.
pub fn metric(label: &'static str, value: String, accent: bool) -> impl IntoElement {
    card(
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_xs().text_color(t::muted()).child(label))
            .child(
                div()
                    .text_2xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .when(accent, |el| el.text_color(t::accent()))
                    .child(value),
            ),
    )
}

/// Engine state as a coloured pill.
pub fn status_pill(state: &str) -> impl IntoElement {
    let color = match state {
        "Running" => t::ok(),
        "Error" => t::danger(),
        "Starting" | "Stopping" | "Connecting" => t::warn(),
        _ => t::muted(),
    };

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .px_3()
        .py_1()
        .rounded_full()
        .border_1()
        .border_color(color)
        .child(div().text_color(color).child("●"))
        .child(div().text_color(color).child(state.to_string()))
}

pub fn fmt_rate(bytes_per_sec: f64) -> String {
    let unit = if bytes_per_sec >= 1024.0 * 1024.0 * 1024.0 {
        (1024.0 * 1024.0 * 1024.0, "GiB/s")
    } else if bytes_per_sec >= 1024.0 * 1024.0 {
        (1024.0 * 1024.0, "MiB/s")
    } else if bytes_per_sec >= 1024.0 {
        (1024.0, "KiB/s")
    } else {
        (1.0, "B/s")
    };
    format!("{:.2} {}", bytes_per_sec / unit.0, unit.1)
}

pub fn fmt_bytes(bytes: u64) -> String {
    fmt_total(bytes as f64)
}

fn fmt_total(bytes: f64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}
