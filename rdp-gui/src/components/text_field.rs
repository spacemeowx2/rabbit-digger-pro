//! A minimal single-line text field, shared by both targets.
//!
//! GPUI core ships the platform plumbing (`EntityInputHandler`) but no widget
//! library, so the settings page gets its own field instead of pulling in a
//! component crate. The cursor is always at the end: enough for URLs, ports and
//! addresses, and much less code than a full editing model.

use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};

use gpui::{
    canvas, div, prelude::*, App, Bounds, Context, ElementInputHandler, EntityInputHandler,
    FocusHandle, IntoElement, KeyDownEvent, Pixels, Point, Render, SharedString, UTF16Selection,
    Window,
};

use crate::theme as t;

static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

pub struct TextField {
    id: usize,
    text: String,
    placeholder: SharedString,
    focus: FocusHandle,
    /// UTF-8 byte range of the IME composition, when one is active.
    marked: Option<Range<usize>>,
    on_submit: Option<Box<dyn Fn(&str, &mut Window, &mut App) + 'static>>,
}

impl TextField {
    pub fn new(
        placeholder: impl Into<SharedString>,
        initial: impl Into<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            text: initial.into(),
            placeholder: placeholder.into(),
            focus: cx.focus_handle().tab_stop(true),
            marked: None,
            on_submit: None,
        }
    }

    pub fn on_submit(mut self, callback: impl Fn(&str, &mut Window, &mut App) + 'static) -> Self {
        self.on_submit = Some(Box::new(callback));
        self
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let command = event.keystroke.modifiers.control || event.keystroke.modifiers.platform;

        // Native platforms deliver paste as a key event, not through
        // `EntityInputHandler::paste` (the web does the latter).
        if command && event.keystroke.key.eq_ignore_ascii_case("v") {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                let text: String = text
                    .chars()
                    .map(|ch| if ch == '\n' || ch == '\r' || ch == '\t' { ' ' } else { ch })
                    .collect();
                self.replace_text_in_range(None, &text, window, cx);
            }
            return;
        }

        match event.keystroke.key.as_str() {
            "backspace" => {
                self.marked = None;
                self.text.pop();
                window.notify_text_input_changed(&self.focus);
                cx.notify();
            }
            "enter" => {
                if let Some(callback) = self.on_submit.take() {
                    let text = self.text.clone();
                    callback(&text, window, cx);
                    self.on_submit = Some(callback);
                }
            }
            "escape" => window.blur(cx),
            _ => {}
        }
    }

    fn marked_utf16(&self) -> Option<Range<usize>> {
        self.marked.as_ref().map(|range| {
            let start = utf16_len(&self.text[..range.start.min(self.text.len())]);
            let end = utf16_len(&self.text[..range.end.min(self.text.len())]);
            start..end
        })
    }
}

impl Render for TextField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.is_focused(window);
        let empty = self.text.is_empty();
        let shown = if empty && !focused {
            self.placeholder.to_string()
        } else if focused {
            format!("{}|", self.text)
        } else {
            self.text.clone()
        };

        let view = cx.entity();
        let focus = self.focus.clone();

        div()
            .relative()
            .id(("text-field", self.id))
            .track_focus(&self.focus)
            .cursor_text()
            .on_click(cx.listener(|this, _event, window, cx| {
                window.focus(&this.focus, cx);
            }))
            .on_key_down(cx.listener(|this, event, window, cx| {
                this.key_down(event, window, cx);
            }))
            .child(
                div()
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .border_color(if focused { t::accent() } else { t::border() })
                    .bg(t::window_bg())
                    .text_xs()
                    .text_color(if empty && !focused {
                        t::muted()
                    } else {
                        t::text()
                    })
                    .child(shown),
            )
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        window.handle_input(
                            &focus,
                            ElementInputHandler::new(bounds, view.clone()),
                            cx,
                        );
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
    }
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// Converts a UTF-16 offset to a UTF-8 byte offset, clamped to a char boundary.
fn utf16_to_utf8(text: &str, offset: usize) -> usize {
    let mut units = 0;
    for (index, ch) in text.char_indices() {
        if units >= offset {
            return index;
        }
        units += ch.len_utf16();
    }
    text.len()
}

fn utf16_range(text: &str, range: &Range<usize>) -> Range<usize> {
    let start = range.start.min(text.len());
    let end = range.end.min(text.len());
    utf16_len(&text[..start])..utf16_len(&text[..end])
}

/// Replaces a UTF-16 range with `new_text`. During IME composition the
/// replacement becomes the new marked range, so the next composition step
/// replaces it instead of appending to it.
fn replace_text(
    text: &mut String,
    marked: &mut Option<Range<usize>>,
    range_utf16: Option<Range<usize>>,
    new_text: &str,
    keep_marked: bool,
) {
    let marked_utf16 = marked.as_ref().map(|range| utf16_range(text, range));
    let range_utf16 = range_utf16
        .or(marked_utf16)
        .unwrap_or_else(|| utf16_len(text)..utf16_len(text));
    let start = utf16_to_utf8(text, range_utf16.start);
    let end = utf16_to_utf8(text, range_utf16.end).max(start);
    text.replace_range(start..end, new_text);
    *marked = if keep_marked && !new_text.is_empty() {
        Some(start..start + new_text.len())
    } else {
        None
    };
}

impl EntityInputHandler for TextField {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let start = utf16_to_utf8(&self.text, range_utf16.start);
        let end = utf16_to_utf8(&self.text, range_utf16.end).max(start);
        let start_utf16 = utf16_len(&self.text[..start]);
        let end_utf16 = start_utf16 + utf16_len(&self.text[start..end]);
        *adjusted_range = Some(start_utf16..end_utf16);
        Some(self.text[start..end].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let end = utf16_len(&self.text);
        Some(UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_utf16()
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        replace_text(&mut self.text, &mut self.marked, range_utf16, text, false);
        window.notify_text_input_changed(&self.focus);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        replace_text(
            &mut self.text,
            &mut self.marked,
            range_utf16,
            new_text,
            true,
        );
        window.notify_text_input_changed(&self.focus);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(element_bounds)
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(utf16_len(&self.text))
    }

    fn accepts_text_input(&self, _window: &mut Window, _cx: &mut Context<Self>) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_round_trip_through_utf16() {
        let text = "http://例え.jp/订阅";
        for (offset, _) in text.char_indices() {
            assert_eq!(utf16_to_utf8(text, utf16_len(&text[..offset])), offset);
        }
        assert_eq!(utf16_to_utf8(text, 0), 0);
        assert_eq!(utf16_to_utf8(text, usize::MAX), text.len());
    }

    #[test]
    fn ime_composition_replaces_its_marked_text() {
        let mut text = String::from("ab");
        let mut marked = None;

        // The IME sends the whole composition on each step.
        replace_text(&mut text, &mut marked, None, "你", true);
        assert_eq!(text, "ab你");
        assert_eq!(marked, Some(2..5));

        replace_text(&mut text, &mut marked, None, "你好", true);
        assert_eq!(text, "ab你好");
        assert_eq!(marked, Some(2..8));

        // Committing the composition keeps the final text and clears the mark.
        let range = marked.as_ref().map(|range| utf16_range(&text, range));
        replace_text(&mut text, &mut marked, range, "你好", false);
        assert_eq!(text, "ab你好");
        assert_eq!(marked, None);

        replace_text(&mut text, &mut marked, None, "!", false);
        assert_eq!(text, "ab你好!");
    }
}
