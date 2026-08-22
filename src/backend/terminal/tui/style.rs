//! TUI styling

use ratatui::style::{Color, Modifier, Style};

pub static STYLES: Styles = Styles::new();

/// Global hard-coded TUI styles
pub struct Styles {
    /// Subheader within a panel
    pub subheader: Style,
    /// Label for a chunk of memory (e.g `ROM`)
    pub memory_range_label: Style,
    /// Memory addresses in the gutter of the memory panel
    pub memory_gutter: Style,
    /// Additional styles for bytes part of the active instruction
    pub memory_pc: Style,
}

impl Styles {
    const fn new() -> Self {
        Self {
            subheader: Style::new().add_modifier(Modifier::UNDERLINED),
            memory_range_label: Style::new().fg(Color::Gray),
            memory_gutter: Style::new().add_modifier(Modifier::BOLD),
            memory_pc: Style::new().add_modifier(Modifier::UNDERLINED),
        }
    }

    /// Get text styling for a boolean flag
    pub fn bool(&self, value: bool) -> Style {
        if value { Color::Green } else { Color::Red }.into()
    }

    /// Get text styling for an 8-bit value
    ///
    /// This provides some visual guidance when reading bytes.
    /// https://simonomi.dev/blog/color-code-your-bytes/
    pub fn u8(&self, value: u8) -> Style {
        // https://github.com/simonomi/hexapoda/blob/bf8bd6297d649b3fb1f100bdc99272705fa558b3/src/buffer/widget/hex.rs#L210
        let color = match value {
            0x00 => Color::Rgb(0x80, 0x80, 0x80), // grey
            0x01..0x10 => Color::Rgb(0xFF, 0x71, 0xA9), // red
            0x10..0x20 => Color::Rgb(0xFF, 0x7A, 0x78), // salmon
            0x20..0x30 => Color::Rgb(0xFF, 0x81, 0x23), // red-orange
            0x30..0x40 => Color::Rgb(0xF7, 0x93, 0x00), // yellow-orange
            0x40..0x50 => Color::Rgb(0xE6, 0x9F, 0x00), // yellow
            0x50..0x60 => Color::Rgb(0xC1, 0xB2, 0x00), // green-yellow
            0x60..0x70 => Color::Rgb(0x82, 0xC6, 0x00), // lime
            0x70..0x80 => Color::Rgb(0x00, 0xD5, 0x00), // green
            0x80..0x90 => Color::Rgb(0x00, 0xD4, 0x59), // clover
            0x90..0xA0 => Color::Rgb(0x00, 0xD0, 0x91), // teal
            0xA0..0xB0 => Color::Rgb(0x00, 0xCC, 0xBB), // cyan
            0xB0..0xC0 => Color::Rgb(0x00, 0xC7, 0xDE), // light blue
            0xC0..0xD0 => Color::Rgb(0x00, 0xBE, 0xFF), // blue
            0xD0..0xE0 => Color::Rgb(0x6C, 0xAF, 0xFF), // blurple
            0xE0..0xF0 => Color::Rgb(0xB2, 0x98, 0xFF), // purple
            0xF0..0xFF => Color::Rgb(0xFF, 0x4D, 0xFF), // pink
            0xFF => Color::White,
        };
        Style::new().fg(color)
    }

    /// Get colorful text styling for a 16-bit value
    ///
    /// This provides some visual guidance when reading bytes.
    /// https://simonomi.dev/blog/color-code-your-bytes/
    pub fn u16(&self, value: u16) -> Style {
        self.u8(value.to_be_bytes()[0])
    }
}
