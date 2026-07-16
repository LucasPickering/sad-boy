//! Graphics processing
//!
//! This computes graphics output and sends it to the virtual screen. The
//! physical drawing is done in [crate::screen].

use crate::{
    emu::{
        cpu::Cycles,
        memory::{self, Memory},
    },
    util::{Bit, BitPack, Mask, PackedBits, impl_bit_pack},
};
use std::{fmt::Debug, mem};

const DOTS_PER_SCANLINE: u32 = 456;
const SCANLINES_PER_FRAME: u32 = 154;

// Const assertions make the unsafe code a bit more safe
const _: () = assert!(mem::size_of::<ObjectAttributes>() == 4);
const _: () = assert!(mem::size_of::<Tile>() == 16);
const _: () = assert!(mem::size_of::<TileIndex>() == 1);

/// Graphics registers and processing
#[derive(Debug)]
pub struct Gpu {
    /// 1-byte control registers related to graphics processing
    registers: Registers,
    ppu: Ppu,
    /// Object Attribute Memory
    ///
    /// https://gbdev.io/pandocs/OAM.html
    oam: Memory<ObjectAttributes>,
    /// Pixel data for tiles
    ///
    /// https://gbdev.io/pandocs/Tile_Data.html
    tile_data: Memory<Tile>,
    /// Two 32x32 tile maps
    ///
    /// The first half of the block is the lower tile map; second half is the
    /// upper tile map.
    ///
    /// https://gbdev.io/pandocs/Tile_Maps.html
    tile_maps: Memory<TileIndex>,
}

impl Gpu {
    /// Advance the current frame draw a certain number of dots
    pub fn execute(&mut self, dots: Cycles) {
        self.ppu.execute(dots);
    }

    /// Get GPU registers
    pub fn registers(&self) -> &Registers {
        &self.registers
    }

    /// Get a mutable reference to GPU registers
    pub fn registers_mut(&mut self) -> &mut Registers {
        &mut self.registers
    }

    /// Get a reference to Object Attribute Memory
    pub fn oam(&self) -> &Memory<ObjectAttributes> {
        &self.oam
    }

    /// Get a mutable reference to Object Attribute Memory
    pub fn oam_mut(&mut self) -> &mut Memory<ObjectAttributes> {
        &mut self.oam
    }

    /// Get a reference to tile pixel data VRAM
    pub fn tile_data(&self) -> &Memory<Tile> {
        &self.tile_data
    }

    /// Get a mutable reference to tile pixel data VRAM
    pub fn tile_data_mut(&mut self) -> &mut Memory<Tile> {
        &mut self.tile_data
    }

    /// Get a reference to tile maps VRAM
    pub fn tile_maps(&self) -> &Memory<TileIndex> {
        &self.tile_maps
    }

    /// Get a mutable reference to tile maps VRAM
    pub fn tile_maps_mut(&mut self) -> &mut Memory<TileIndex> {
        &mut self.tile_maps
    }
}

impl Default for Gpu {
    fn default() -> Self {
        Self {
            registers: Registers::default(),
            ppu: Ppu::default(),
            oam: Memory::new(memory::OAM),
            tile_data: Memory::new(memory::TILE_DATA),
            tile_maps: Memory::new(memory::TILE_MAPS),
        }
    }
}

/// Registers in the GPU
///
/// This is a subset of the [hardware register list](https://gbdev.io/pandocs/Hardware_Reg_List.html).
/// These can be modified via the memory bus.
#[derive(Debug, Default)]
pub struct Registers {
    /// OAM DMA control
    ///
    /// The written value is the **high** byte of the transfer source address.
    /// Only values `0x00` to `0xDF` are valid.
    pub dma: u8,
    /// LCD control
    pub lcdc: u8,
    /// LCD status
    pub stat: u8,
    /// Viewport scroll X
    pub scx: u8,
    /// Viewport scroll Y
    pub scy: u8,
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

/// An 8x8 collection of pixels
///
/// https://gbdev.io/pandocs/Tile_Data.html
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)] // Memory layout matters here
pub struct Tile {
    /// A tile is 16 bytes:
    /// - 4 colors per pixel => 2 bits per pixel
    /// - 8 pixels per line => 2 bytes per line
    /// - 8 lines => 16 bytes total
    ///
    /// The pixel layout is a little odd: each pixel's bits are split across
    /// both bytes of that line. For a given line, bit 7 of each byte specifies
    /// the left-most pixel, bit 6 is the second pixel, etc. The first byte
    /// holds the lesser bit, second byte holds the greater bit.
    lines: [(u8, u8); 8],
}

/// Metadata specifying a single pixel object
///
/// https://gbdev.io/pandocs/OAM.html
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)] // Memory layout matters here
pub struct ObjectAttributes {
    // Field order must match the doc above
    /// Vertical position of the object + 16
    y: u8,
    /// Horizontal position of the object + 8
    x: u8,
    /// Index of the tile defining this object
    ///
    /// For 8x8 tiles, this is the index into the tile map for the object's
    /// only tile. For 8x16 tiles, it's the index of the first (upper) tile,
    /// and the lower tile is the subsequent tile in the map.
    tile_index: TileIndex,
    /// TODO
    flags: PackedBits<ObjectFlags>,
}

/// Index of a single tile in a tile map
///
/// https://gbdev.io/pandocs/Tile_Maps.html#tile-indexes
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct TileIndex(u8);

/// Flags in byte 3 of [ObjectAttributes]
///
/// This is packed as a single byte in memory; this struct is the unpacked
/// semantic data.
///
/// https://gbdev.io/pandocs/OAM.html#byte-3--attributesflags
struct ObjectFlags {
    cgb_palette: CgbPalette,
    bank: VramBank,
    dmg_palette: DmgPalette,
    x_flip: bool,
    y_flip: bool,
    priority: bool,
}

impl_bit_pack! {
    ObjectFlags;
    Mask::M210 => cgb_palette,
    Bit(3).mask() => bank,
    Bit(4).mask() => dmg_palette,
    Bit(5).mask() => x_flip,
    Bit(6).mask() => y_flip,
    Bit(7).mask() => priority,
}

impl BitPack for bool {
    fn from_bits(bits: u8) -> Self {
        (bits & 0b1) == 1
    }

    fn to_bits(&self) -> u8 {
        (*self).into()
    }
}

enum CgbPalette {
    Obp0,
    Obp1,
    Obp2,
    Obp3,
    Obp4,
    Obp5,
    Obp6,
    Obp7,
}

impl BitPack for CgbPalette {
    fn from_bits(bits: u8) -> Self {
        match bits & 0b111 {
            0b000 => Self::Obp0,
            0b001 => Self::Obp1,
            0b010 => Self::Obp2,
            0b011 => Self::Obp3,
            0b100 => Self::Obp4,
            0b101 => Self::Obp5,
            0b110 => Self::Obp6,
            0b111 => Self::Obp7,
            _ => unreachable!(),
        }
    }

    fn to_bits(&self) -> u8 {
        match self {
            Self::Obp0 => 0b000,
            Self::Obp1 => 0b001,
            Self::Obp2 => 0b010,
            Self::Obp3 => 0b011,
            Self::Obp4 => 0b100,
            Self::Obp5 => 0b101,
            Self::Obp6 => 0b110,
            Self::Obp7 => 0b111,
        }
    }
}

enum VramBank {
    Bank0,
    Bank1,
}

impl BitPack for VramBank {
    fn from_bits(bits: u8) -> Self {
        match bits & 0b1 {
            0b0 => Self::Bank0,
            0b1 => Self::Bank1,
            _ => unreachable!(),
        }
    }

    fn to_bits(&self) -> u8 {
        match self {
            Self::Bank0 => 0b0,
            Self::Bank1 => 0b1,
        }
    }
}

enum DmgPalette {
    Obp0,
    Obp1,
}

impl BitPack for DmgPalette {
    fn from_bits(bits: u8) -> Self {
        match bits & 0b1 {
            0b0 => Self::Obp0,
            0b1 => Self::Obp1,
            _ => unreachable!(),
        }
    }

    fn to_bits(&self) -> u8 {
        match self {
            Self::Obp0 => 0b0,
            Self::Obp1 => 0b1,
        }
    }
}
