use rd_interface::{config::NetRef, prelude::*, Address};

#[rd_config]
#[derive(Debug, Clone)]
pub struct Hysteria2Config {
    /// Server address
    pub server: Address,

    /// Authentication string
    #[serde(skip_serializing_if = "rd_interface::config::detailed_field")]
    pub auth: String,

    /// SNI for TLS handshake
    #[serde(default)]
    pub sni: Option<String>,

    /// Skip certificate verification
    #[serde(default)]
    pub skip_cert_verify: bool,

    /// Client receive window size in bytes
    #[serde(default = "default_rx_window")]
    pub rx_window: u32,

    /// Disable UDP support
    #[serde(default)]
    pub disable_udp: bool,

    /// Optional obfuscation password (Salamander)
    #[serde(default)]
    pub obfs: Option<String>,

    /// Upstream network for outbound connections
    #[serde(default)]
    pub net: NetRef,
}

fn default_rx_window() -> u32 {
    // Default to 64MB
    64 * 1024 * 1024
}
