mod api;
mod app;
mod pages;
mod store;
mod theme;

use gpui::{px, size, App, AppContext, Bounds, QuitMode, WindowBounds, WindowOptions};

/// Where the JSON-RPC daemon listens. Native builds can override it with
/// `RD_API`; a web build reads `?api=` from the page URL.
fn endpoint() -> String {
    #[cfg(not(target_family = "wasm"))]
    {
        std::env::var("RD_API").unwrap_or_else(|_| api::DEFAULT_ENDPOINT.to_string())
    }

    #[cfg(target_family = "wasm")]
    {
        api::DEFAULT_ENDPOINT.to_string()
    }
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
            let bounds = Bounds::centered(None, size(px(1120.), px(760.)), cx);
            cx.open_window(
                WindowOptions::default().window_bounds(WindowBounds::Windowed(bounds)),
                |_window, cx| cx.new(|cx| app::Root::new(endpoint.clone(), cx)),
            )
            .expect("failed to open the main window");
            cx.activate(true);
        });
}
