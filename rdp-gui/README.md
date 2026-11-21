# rdp-gui

The desktop and browser UI for rabbit-digger-pro, written with
[GPUI](https://crates.io/crates/gpui-ce) (the community edition of Zed's UI
framework).

One set of views drives both targets. Only the socket differs: `api/native.rs`
uses tokio-tungstenite, `api/web.rs` uses `web_sys::WebSocket`, and both hand
the same events to the `Store` entity in `store.rs`.

## Talk to the daemon

The UI is a client of the JSON-RPC API served by `rabbit-digger-pro --bind`.
Nothing is shared in-process, so the same binary can drive a local or a remote
engine.

```sh
# terminal 1: the engine
cargo run -- --config config.yaml --bind 127.0.0.1:9091

# terminal 2: the window
cargo run -p rdp-gui
```

The endpoint defaults to `127.0.0.1:9091`; override it with `RD_API`
(`RD_API=192.168.1.10:9091 cargo run -p rdp-gui`).

Pages: **状态** (status and traffic), **连接** (live connections), **日志**
(log stream), **节点** (switch a net's node), **设置** (engine controls and the
current config).

## Build for the browser

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk
cd rdp-gui
trunk serve            # dev server on http://127.0.0.1:8080
trunk build --release  # static bundle in rdp-gui/dist
```

The page talks to the daemon that served it, so serving `dist` from the daemon
keeps everything on one origin:

```sh
cargo run -- --config config.yaml --bind 127.0.0.1:9091 --web-ui rdp-gui/dist
```

Use `?api=host:port` to point a page at a different daemon.

## Fonts in the web build

The browser exposes no system fonts, so the wasm build carries its own subset
of Noto Sans SC (SIL OFL, see `assets/fonts/LICENSE-OFL.txt`) and registers it
at startup. Native builds keep using the operating system's fonts.

GPUI's default family is `.SystemUIFont`, which the platform resolves to a
generic system family first and to a registered face second. On the web that
first step matches nothing, so the views name the embedded family
(`RdpUI`, `main.rs::UI_FAMILY`) explicitly instead of relying on the default.

The subset only covers the characters the UI actually draws. After changing UI
text, rebuild it:

```sh
python scripts/build-font-subset.py path/to/NotoSansSC-Regular.otf
```

That writes `assets/fonts/RdpUI.otf` and `assets/fonts/glyphs.txt` (the list of
covered characters). Without a glyph the browser draws a missing-glyph box.

## Known limits of the web target

These come from GPUI's browser backend, not from this crate:

- One window per page, drawn into a single `<canvas>`; window size, position and
  window kinds are ignored, and a second window fails.
- No system font lookup, so every font must be embedded before drawing text.
- No accessibility tree, and no read access to the clipboard (`read_from_clipboard`
  is always `None`; use the async variant).
- No file dialogs, notifications, menus or screen capture.

The `multithreaded` feature of `gpui_ce_platform` is deliberately off: it needs a
nightly `build-std` toolchain. A single-threaded wasm build runs on stable.
