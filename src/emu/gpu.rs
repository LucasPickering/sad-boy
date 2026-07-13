//! Graphics processing
//!
//! This computes graphics output and sends it to the virtual screen. The
//! physical drawing is done in [crate::screen].

use crate::emu::cpu::Cycles;

const DOTS_PER_SCANLINE: u32 = 456;
const SCANLINES_PER_FRAME: u32 = 154;

/// Pixel Processing Unit
///
/// This controls the rendering state within a single frame. A frame consists
/// of 154 scanlines, each taking 456 dots.
///
/// This page is really good: https://gbdev.io/pandocs/Rendering.html
#[derive(Debug, Default)]
pub struct Ppu {
    /// Number of elapsed dots in the current frame
    ///
    /// TODO don't store this here, since it's stored in the parent as well
    dots: Cycles,
    mode: PpuMode,
}

impl Ppu {
    /// Advance the current frame draw a certain number of dots
    pub fn execute(&mut self, dots: Cycles) {
        self.dots.0 += dots.0;
        let scanline = self.scanline();
        self.mode = match scanline {
            // We're in one of the drawing scanlines - figure out where in the
            // scanline we are
            0..144 => match self.dots.0 % DOTS_PER_SCANLINE {
                0..80 => PpuMode::OamScan,
                // TODO figure out how to makes modes 3/0 dynamic
                80..252 => PpuMode::HorizontalBlank,
                252..DOTS_PER_SCANLINE => PpuMode::Drawing,
                // This is impossible because of the modulo above
                DOTS_PER_SCANLINE.. => unreachable!(
                    "scanline cannot have more than {DOTS_PER_SCANLINE} dots"
                ),
            },
            144..SCANLINES_PER_FRAME => PpuMode::VerticalBlank,
            // This indicates there were too many dots in a frame. That *should*
            // be impossible because the longest CPU instruction is 8 cycles,
            // and 456 is divisible by 8. Indicates a bug somewhere.
            SCANLINES_PER_FRAME.. => panic!(
                "frame cannot have more than {SCANLINES_PER_FRAME} scanlines, \
                but scanline index is {scanline} ({dots:?} dots)",
                dots = self.dots
            ),
        };
    }

    /// Number of the scanline last drawn to
    ///
    /// The first scanline of a frame is `0`; the last is `153`.
    fn scanline(&self) -> u32 {
        self.dots.0 / DOTS_PER_SCANLINE
    }
}

/// Draw mode within the current frame
///
/// This defines what the PPU is doing within a single frame draw.
/// https://gbdev.io/pandocs/Rendering.html#ppu-modes
#[derive(Debug, Default)]
enum PpuMode {
    /// Mode 0
    ///
    /// The tail end of a scan line, when the PPU is just waiting for the next
    /// scan line to begin.
    HorizontalBlank,
    /// Mode 1
    ///
    /// The tail end of the entire frame.
    VerticalBlank,
    /// Mode 2
    #[default]
    OamScan,
    /// Mode 3
    Drawing,
}
