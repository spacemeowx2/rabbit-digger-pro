mod api;
mod app;
mod pages;
mod store;
mod theme;

use gpui::{px, size, App, AppContext, Bounds, QuitMode, WindowBounds, WindowOptions};

/// Family name of the embedded font. Must match the name table of `UI_FONT`.
/// GPUI's own default family (`.SystemUIFont`) cannot resolve in a browser: the
/// platform maps it to a generic system font and that lookup has no fallback
/// there, so the wasm build names this face explicitly instead.
#[cfg(target_family = "wasm")]
pub const UI_FAMILY: &str = "RdpUI";

/// Subset of Noto Sans SC registered as `UI_FAMILY`. Only the wasm build
/// needs it: native backends read the operating system's fonts.
#[cfg(target_family = "wasm")]
const UI_FONT: &[u8] = include_bytes!("../assets/fonts/RdpUI.otf");

/// Where the JSON-RPC daemon listens. Native builds can override it with
/// `RD_API`; a web build reads `?api=` from the page URL and otherwise talks to
/// whatever host served the page.
fn endpoint() -> String {
    #[cfg(not(target_family = "wasm"))]
    {
        std::env::var("RD_API").unwrap_or_else(|_| api::DEFAULT_ENDPOINT.to_string())
    }

    #[cfg(target_family = "wasm")]
    {
        use wasm_bindgen::JsCast;

        let Some(window) = web_sys::window() else {
            return api::DEFAULT_ENDPOINT.to_string();
        };
        let location = window.location();

        let search = location.search().unwrap_or_default();
        for pair in search.trim_start_matches('?').split('&') {
            if let Some(value) = pair.strip_prefix("api=") {
                if !value.is_empty() {
                    return percent_decode(value);
                }
            }
        }

        // Served by the daemon itself: same host, so the socket goes there too.
        let host = location.host().unwrap_or_default();
        if !host.is_empty() {
            return host;
        }
        let _ = window.dyn_ref::<web_sys::Window>();
        api::DEFAULT_ENDPOINT.to_string()
    }
}

#[cfg(target_family = "wasm")]
fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("00");
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    out.push(byte);
                    index += 3;
                    continue;
                }
                out.push(bytes[index]);
                index += 1;
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn main() {
    #[cfg(target_family = "wasm")]
    {
        console_error_panic_hook::set_once();
        gpui_platform::web_init();
    }

    let endpoint = endpoint();

    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(move |cx: &mut App| {
            // Browsers expose no system fonts and GPUI's default family is
            // `.SystemUIFont`, so the wasm build carries its own subset. See
            // `scripts/build-font-subset.py`.
            #[cfg(target_family = "wasm")]
            cx.text_system()
                .add_fonts(vec![std::borrow::Cow::Borrowed(UI_FONT)])
                .expect("the embedded UI font is valid");

            let bounds = Bounds::centered(None, size(px(1120.), px(760.)), cx);
            cx.open_window(
                WindowOptions::default().window_bounds(WindowBounds::Windowed(bounds)),
                |_window, cx| cx.new(|cx| app::Root::new(endpoint.clone(), cx)),
            )
            .expect("failed to open the main window");
            cx.activate(true);
        });
}
