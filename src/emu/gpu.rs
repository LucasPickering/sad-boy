//! Graphics processing
//!
//! This computes graphics output and sends it to the virtual screen. The
//! physical drawing is done in [crate::screen].

use crate::{
    emu::{
        cpu::Cycles,
        memory::{self, Memory},
    },
    screen::Screen,
    util::{Bit, Mask, PackedBits, impl_bit_pack},
};
use std::{fmt::Debug, mem};

const DOTS_PER_SCANLINE: u32 = 456;
const SCANLINES_PER_FRAME: u32 = 154;

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
    /// This is split into 3 logical blocks, each 128 tiles (2048 bytes).
    /// At any given time, two blocks are accessible (0-1 or 1-2) based on
    /// bit 4 of the `LCDC` register. See [TileDataArea] for more.
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
    pub async fn run(self, screen: &mut Screen) {
        todo!()
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

    /// Get a list of **up to 10** visible objects for the current scanline
    ///
    /// When there are more than 10 objects intersecting with the current
    /// scanline, the objects earlier in memory (with lower addresses) get
    /// priority.
    ///
    /// https://gbdev.io/pandocs/OAM.html#selection-priority
    fn get_objects(&self) -> Vec<ObjectAttributes> {
        let line = self.registers.ly;
        let object_height = self.registers.lcdc.unpack().object_size.height();
        // Take the first 10 objects intersecting the current line
        //
        self.oam
            .as_values()
            .iter()
            .filter(|object| object.intersects(line, object_height))
            .take(10)
            .copied()
            .collect()
    }

    /// TODO
    fn get_tile(&self, index: TileIndex) -> &Tile {
        // Select active tiles based on the LCDC flag
        let tiles =
            self.get_tiles(self.registers.lcdc.unpack().bg_window_tiles);
        // Safety: tiles is an array of 256, so the index must be valid
        &tiles[index.0 as usize]
    }

    /// Get the block of accessible tiles for the given addressing mode
    ///
    ///
    /// Each addressing mode can access exactly 256 tiles, so that's encoded in
    /// the return type.
    fn get_tiles(&self, area: TileDataArea) -> &[Tile; 256] {
        let tiles = self.tile_data.as_values();
        debug_assert_eq!(
            tiles.len(),
            128 * 3,
            "Tile data should be 3 blocks of 128 tiles"
        );
        let slice = match area {
            TileDataArea::Low => &tiles[..256],
            TileDataArea::High => &tiles[128..],
        };
        slice.try_into().expect("256 tiles accessible at a time")
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
    pub lcdc: PackedBits<LcdControl>,
    /// LCD status
    pub stat: PackedBits<LcdStatus>,
    /// Viewport scroll X
    pub scx: u8,
    /// Viewport scroll Y
    pub scy: u8,
    /// Current horizontal line being drawn on the LCD (**read-only**)
    ///
    /// Range is `[0, 153]`. `[144, 153]` is the vblank period.
    pub ly: Scanline,
    /// A writable register compared to `LY` every cycle
    ///
    /// When `LY == LYC`, bit 2 of the `STAT` register is set. See [LcdStatus].
    pub lyc: Scanline,
}

/// Bit-packed values in the `LCDC` register
///
/// https://gbdev.io/pandocs/LCDC.html
#[derive(Debug)]
pub struct LcdControl {
    /// Are the LCD and PPU enabled?
    lcd_enable: bool,
    /// Tile map in use for the window
    window_tile_map: TileMapArea,
    ///
    ///
    /// It's complicated - see the Pandocs
    window_enable: bool,
    /// Which blocks are accessible for background and window tiles?
    ///
    /// Objects are unaffected by this. They always use the low area.
    bg_window_tiles: TileDataArea,
    /// Tile map in use for the background
    bg_tile_map: TileMapArea,
    /// Size of the next object to draw
    object_size: ObjectSize,
    /// TODO
    object_enable: bool,
    /// Disable the background AND window
    ///
    /// If zero, the `window_enable` flag is ignored. On CGB, this is actually
    /// the `priority` flag.
    ///
    /// It's complicated - see the Pandocs
    bg_window_enable: bool,
}

impl_bit_pack! {
    struct LcdControl;
    Bit(7).mask() => lcd_enable,
    Bit(6).mask() => window_tile_map,
    Bit(5).mask() => window_enable,
    Bit(4).mask() => bg_window_tiles,
    Bit(3).mask() => bg_tile_map,
    Bit(2).mask() => object_size,
    Bit(1).mask() => object_enable,
    Bit(0).mask() => bg_window_enable,
}

/// Selector for a block of tile map memory
///
/// Used for multiple flags in [LcdControl].
#[derive(Debug)]
enum TileMapArea {
    /// `0x9800–0x9BFF`
    Low,
    /// `0x9C00–0x9FFF`
    High,
}

impl_bit_pack! {
    enum TileMapArea;
    0b0 => Low,
    0b1 => High,
}

/// Selector for which blocks of tile data are in use.
///
/// There are 3 blocks:
/// - Block 0: `0x8000-0x87FF`
/// - Block 1: `0x8800-0x8FFF`
/// - Block 2: `0x9000-0x97FF`
///
/// At any given time two blocks are accessible: 0-1 or 1-2.
#[derive(Debug)]
enum TileDataArea {
    /// `0x8000-0x8FFF` (blocks 0 and 1)
    ///
    /// This is called "`$8000` addressing mode" in Pandocs
    Low,
    /// `0x8800-0x97FF` (blocks 1 and 2)
    ///
    /// This is called "`$8800` addressing mode" in Pandocs
    High,
}

impl_bit_pack! {
    enum TileDataArea;
    // Backwards!
    0b0 => High,
    0b1 => Low,
}

/// Size of the next object to draw (flag in [LcdControl])
#[derive(Debug)]
enum ObjectSize {
    /// 8x8
    Small,
    /// 8x16
    Large,
}
impl ObjectSize {
    fn height(&self) -> u8 {
        match self {
            ObjectSize::Small => 8,
            ObjectSize::Large => 16,
        }
    }
}

impl_bit_pack! {
    enum ObjectSize;
    0b0 => Small,
    0b1 => Large,
}

/// Bit-packed values in the `STAT` register
///
/// https://gbdev.io/pandocs/STAT.html
#[derive(Debug)]
pub struct LcdStatus {
    /// TODO
    lyc_interrupt: bool,
    /// TODO
    mode_2_interrupt: bool,
    /// TODO
    mode_1_interrupt: bool,
    /// TODO
    mode_0_interrupt: bool,
    /// Is the `LY` register currently equal to the `LYC` register?
    ///
    /// See [Registers] for those register definitions.
    lyc_equal_ly: bool,
    /// TODO
    ppu_mode: PpuMode,
}

impl_bit_pack! {
    struct LcdStatus;
    // Bit 7 is empty
    Bit(6).mask() => lyc_interrupt,
    Bit(5).mask() => mode_2_interrupt,
    Bit(4).mask() => mode_1_interrupt,
    Bit(3).mask() => mode_0_interrupt,
    Bit(2).mask() => lyc_equal_ly,
    Mask::M10 => ppu_mode,
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

impl_bit_pack! {
    enum PpuMode;
    0b00 => HorizontalBlank,
    0b01 => VerticalBlank,
    0b10 => OamScan,
    0b11 => Drawing,
}

/// Index of a particular horizontal line on the screen
///
/// Range is `[0, 153]`. `[144, 153]` is the vblank period. Any value `>=154` is
/// invalid.
#[derive(Clone, Copy, Debug, Default)]
pub struct Scanline(u8);

/// An 8x8 collection of pixels
///
/// A tile is 16 bytes:
/// - 4 colors per pixel => 2 bits per pixel
/// - 8 pixels per line => 2 bytes per line
/// - 8 lines => 16 bytes total
///
/// https://gbdev.io/pandocs/Tile_Data.html
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)] // Memory layout matters here
pub struct Tile {
    lines: [TileLine; 8],
}
const _: () = assert!(mem::size_of::<Tile>() == 16);

/// A single 8-pixel line in a tile
///
/// A pixel is a color index 0-3 (2 bits). The actual color is defined in a
/// [Palette]. The color index layout is a little odd: each index's bits are
/// split across both bytes of that line. For a given line, bit 7 of each byte
/// specifies the left-most pixel, bit 6 is the second pixel, etc. The first
/// byte holds the lesser bit, second byte holds the greater bit.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
struct TileLine {
    low: u8,
    high: u8,
}
const _: () = assert!(mem::size_of::<TileLine>() == 2);

/// Index of a single tile in a tile map
///
/// https://gbdev.io/pandocs/Tile_Maps.html#tile-indexes
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct TileIndex(u8);
const _: () = assert!(mem::size_of::<TileIndex>() == 1);

/// Metadata specifying a single pixel object
///
/// https://gbdev.io/pandocs/OAM.html
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)] // Memory layout matters here
pub struct ObjectAttributes {
    // Field order must match the doc above
    /// Vertical position of the object + 16
    ///
    /// The +16 allows moving an object above the screen without underflowing
    /// the byte.
    y: u8,
    /// Horizontal position of the object + 8
    ///
    /// The +8 allows moving an object left of the screen without underflowing
    /// the byte.
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
impl ObjectAttributes {
    /// Does this object intersect with the current horizontal line?
    ///
    /// The object height (8 vs 16 pixels) is determined by the `LCDC` register,
    /// so it must be passed in. This *only* checks for vertical intersection.
    /// If an object intersects vertically but is off the screen horizontally,
    /// this will **still return true.** That's consistent with the [object
    /// selection priority algorithm](https://gbdev.io/pandocs/OAM.html#selection-priority).
    fn intersects(self, line: Scanline, object_height: u8) -> bool {
        // self.y is shifted +16. Shift the line up to match. Subtracting could
        // incur underflow. Addition can't overflow because the max line value
        // is 153.
        let line = line.0 + 16;
        let top = self.y; // Top edge (inclusive)
        let bottom = self.y + object_height; // Bottom edge (exclusive)
        bottom > line && top <= line
    }
}
const _: () = assert!(mem::size_of::<ObjectAttributes>() == 4);

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
    struct ObjectFlags;
    Mask::M210 => cgb_palette,
    Bit(3).mask() => bank,
    Bit(4).mask() => dmg_palette,
    Bit(5).mask() => x_flip,
    Bit(6).mask() => y_flip,
    Bit(7).mask() => priority,
}

/// Color palette selection in OAM flags for DMG (original Game Boy) mode
enum DmgPalette {
    Obp0,
    Obp1,
}

impl_bit_pack! {
    enum DmgPalette;
    0b0 => Obp0,
    0b1 => Obp1,
}

/// VRAM bank selection in OAM flags
enum VramBank {
    Bank0,
    Bank1,
}

impl_bit_pack! {
    enum VramBank;
    0b0 => Bank0,
    0b1 => Bank1,
}

/// Color palette selection in OAM flags for CGB (Game Boy Color) mode
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

impl_bit_pack! {
    enum CgbPalette;
    0b000 => Obp0,
    0b001 => Obp1,
    0b010 => Obp2,
    0b011 => Obp3,
    0b100 => Obp4,
    0b101 => Obp5,
    0b110 => Obp6,
    0b111 => Obp7,
}
