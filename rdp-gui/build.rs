fn main() {
    // The MSVC linker embeds a default manifest for GUI-subsystem binaries,
    // which pins the process to DPI-unaware. `main.rs` calls
    // SetProcessDpiAwarenessContext at startup instead; that call is ignored
    // when a manifest already declared awareness, so suppress the default.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        println!("cargo:rustc-link-arg=/MANIFEST:NO");
    }
}
