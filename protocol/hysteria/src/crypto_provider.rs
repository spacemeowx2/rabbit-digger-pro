pub(crate) fn ensure_rustls_provider_installed() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // Quinn uses rustls. With rustls 0.23+, the process-level provider may need
        // to be selected explicitly depending on feature resolution.
        let _ = quinn::rustls::crypto::ring::default_provider().install_default();
    });
}
