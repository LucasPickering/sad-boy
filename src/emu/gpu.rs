//! Graphics processing
//!
//! This computes graphics output and sends it to the virtual screen. The
//! physical drawing is done in [crate::screen].

use crate::emu::{
    cpu::Cycles,
    memory::{Memory, VRAM_LEN},
};

const DOTS_PER_SCANLINE: u32 = 456;
const SCANLINES_PER_FRAME: u32 = 154;

/// Graphics registers and processing
#[derive(Debug, Default)]
pub struct Gpu {
    registers: Registers,
    ppu: Ppu,
    /// Video RAM, containing tiles and background maps
    vram: Memory<VRAM_LEN>,
}

impl Gpu {
    /// Advance the current frame draw a certain number of dots
    pub fn execute(&mut self, dots: Cycles) {
        self.ppu.execute(dots);
    }

    /// Get a reference to Video RAM
    pub fn vram(&self) -> &Memory<VRAM_LEN> {
        &self.vram
    }

    /// Get a mutable reference to Video RAM
    pub fn vram_mut(&mut self) -> &mut Memory<VRAM_LEN> {
        &mut self.vram
    }
}

/// Pixel Processing Unit
///
/// This controls the rendering state within a single frame. A frame consists
/// of 154 scanlines, each taking 456 dots.
///
/// This page is really good: https://gbdev.io/pandocs/Rendering.html
#[derive(Debug, Default)]
struct Ppu {
    /// Number of elapsed dots in the current frame
    ///
    /// TODO don't store this here, since it's stored in the parent as well
    dots: Cycles,
    mode: PpuMode,
}

impl Ppu {
    /// Advance the current frame draw a certain number of dots
    fn execute(&mut self, dots: Cycles) {
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

/// Registers in the GPU
///
/// This is a subset of the [hardware register list](https://gbdev.io/pandocs/Hardware_Reg_List.html).
/// These can be modified via the memory bus.
#[derive(Debug, Default)]
struct Registers {
    /// OAM DMA control
    ///
    /// The written value is the **high** byte of the transfer source address.
    /// Only values `0x00` to `0xDF` are valid.
    dma: u8,
    /// LCD control
    lcdc: u8,
    /// LCD status
    stat: u8,
    /// Viewport scroll X
    scx: u8,
    /// Viewport scroll Y
    scy: u8,
}
