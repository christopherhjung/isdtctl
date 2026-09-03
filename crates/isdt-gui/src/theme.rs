//! The window's colours, in one place.
//!
//! A dark palette, because a charger sits on a bench and the window is often
//! glanced at rather than read.

use gpui::Rgba;

/// Builds a colour from a hex literal at compile time, which `gpui::rgb`
/// cannot do.
const fn hex(value: u32) -> Rgba {
    Rgba {
        r: ((value >> 16) & 0xFF) as f32 / 255.0,
        g: ((value >> 8) & 0xFF) as f32 / 255.0,
        b: (value & 0xFF) as f32 / 255.0,
        a: 1.0,
    }
}

/// Page background.
pub const BACKGROUND: Rgba = hex(0x14161a);
/// Cards and the header bar.
pub const PANEL: Rgba = hex(0x1c1f25);
/// Hairlines.
pub const BORDER: Rgba = hex(0x2a2f37);
/// The empty part of a bar, and unselected chips.
pub const TRACK: Rgba = hex(0x262b33);
/// A chip under the pointer.
pub const HOVER: Rgba = hex(0x323944);

/// Ordinary text.
pub const TEXT: Rgba = hex(0xe6e9ee);
/// Labels and secondary readings.
pub const MUTED: Rgba = hex(0x8b93a1);

/// Charging, and anything else in progress.
pub const ACCENT: Rgba = hex(0x4ade80);
/// The selected chip's background.
pub const ACCENT_DIM: Rgba = hex(0x1e3a2a);
/// Storage tasks.
pub const STORE: Rgba = hex(0x60a5fa);
/// Discharge, and a connection still being established.
pub const WARN: Rgba = hex(0xfbbf24);
/// Faults, and the stop button.
pub const BAD: Rgba = hex(0xf87171);
/// The background behind a fault message.
pub const BAD_BG: Rgba = hex(0x3a1f21);
/// A button that should not shout, such as Disconnect.
pub const NEUTRAL: Rgba = hex(0x323944);

/// A healthy link.
pub const GOOD: Rgba = hex(0x4ade80);
