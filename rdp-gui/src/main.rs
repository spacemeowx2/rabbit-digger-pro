mod api;
mod app;
mod components;
#[cfg(not(target_family = "wasm"))]
mod engine;
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

#[cfg(not(target_family = "wasm"))]
#[derive(clap::Parser)]
#[command(name = "rdp-gui", about = "Rabbit Digger Pro desktop UI")]
struct Args {
    /// Config file the embedded engine loads.
    #[arg(short, long, env = "RD_CONFIG", default_value = "config.yaml")]
    config: std::path::PathBuf,

    /// Attach to an already running daemon instead of starting the engine.
    #[arg(long, env = "RD_API")]
    api: Option<String>,
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
            // Browsers expose no system fonts, so the wasm build carries its
            // own subset. See `scripts/build-font-subset.py`.
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

/// Native builds start the engine in-process and talk to its API; the daemon
/// stays an option for attaching to an existing instance.
#[cfg(not(target_family = "wasm"))]
fn endpoint() -> String {
    let args = <Args as clap::Parser>::parse();
    if let Some(api) = args.api {
        return api;
    }

    engine::init_logging();
    match engine::start(args.config) {
        Ok(addr) => addr.to_string(),
        Err(error) => {
            // Keep the window usable: it shows the connection as failed and
            // the reason in the status page.
            eprintln!("failed to start the embedded engine: {error:#}");
            api::DEFAULT_ENDPOINT.to_string()
        }
    }
}

/// A browser build reads `?api=` from the page URL and otherwise talks to
/// whatever host served the page.
#[cfg(target_family = "wasm")]
fn endpoint() -> String {
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

    api::DEFAULT_ENDPOINT.to_string()
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
