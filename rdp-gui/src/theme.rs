//! One palette for both targets. Kept as plain functions because `rgb` is not
//! a `const fn` in GPUI.

use gpui::{rgb, Rgba};

pub fn window_bg() -> Rgba {
    rgb(0x0f1115)
}

pub fn sidebar_bg() -> Rgba {
    rgb(0x15181e)
}

pub fn card_bg() -> Rgba {
    rgb(0x1a1e26)
}

pub fn border() -> Rgba {
    rgb(0x262b35)
}

pub fn text() -> Rgba {
    rgb(0xe6e9ef)
}

pub fn muted() -> Rgba {
    rgb(0x8b93a1)
}

pub fn accent() -> Rgba {
    rgb(0x5b9dff)
}

pub fn accent_soft() -> Rgba {
    rgb(0x1d2a3f)
}

pub fn ok() -> Rgba {
    rgb(0x3ecf8e)
}

pub fn warn() -> Rgba {
    rgb(0xf5a524)
}

pub fn danger() -> Rgba {
    rgb(0xf06a6a)
}
