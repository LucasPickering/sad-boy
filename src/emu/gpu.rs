//! Graphics processing
//!
//! This computes graphics output and sends it to the virtual screen. The
//! physical drawing is done in [crate::screen].

use crate::{
    emu::{
        Clock, SCREEN_WIDTH,
        cpu::Cycles,
        memory::{self, Memory},
    },
    screen::{Color, Screen},
    util::{Bit, Mask, PackedBits, impl_bit_pack},
};
use std::{fmt::Debug, mem};

const SCANLINES_PER_FRAME: u8 = 154;
/// Number of dots in [PpuMode::OamScan] for a single scanline
const MODE_2_DOTS: Cycles = Cycles(80);

/// Graphics registers and processing
#[derive(Debug)]
pub struct Gpu {
    /// 1-byte control registers related to graphics processing
    registers: Registers,
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
    pub async fn run(mut self, screen: &mut Screen) {
        // Each iteration of this loop is one frame
        //
        // For each frame, this will load the entire frame into the screen's
        // buffer, then draw then entire frame to the screen at the end.
        loop {
            // Make sure the GPU and clock stay in sync
            debug_assert_eq!(
                Clock::elapsed(),
                Cycles(0),
                "Clock should start at 0 for each frame"
            );
            screen.reset();
            self.registers.ly = Scanline(0);

            // TODO keep this in sync with the STAT register
            for scanline in 0..SCANLINES_PER_FRAME {
                let scanline = Scanline(scanline);
                self.draw_scanline(screen, scanline).await;
                self.registers.ly = scanline;
            }
            screen.draw();
        }
    }

    /// Draw a single scanline with the given index to the screen
    ///
    /// https://gbdev.io/pandocs/Rendering.html
    async fn draw_scanline(&mut self, screen: &mut Screen, scanline: Scanline) {
        // TODO wait before or after executing?
        if scanline.0 >= 144 {
            self.set_mode(PpuMode::VerticalBlank);
            Clock::wait(Cycles(456)).await;
        }

        // Mode 2 - OAM scan
        // I didn't find anything in the docs about the actual rate that the GB
        // collects objects per dot, so I'm doing it all up front. This may
        // have a semantic impact, I'm not sure.
        self.set_mode(PpuMode::OamScan);
        let objects = self.get_objects();
        // Render order relies on the objects being sorted
        // NOTE: This is only for non-CGB mode. In CGB mode this will have to
        // change
        debug_assert!(
            objects.is_sorted_by_key(|object| object.attributes.x),
            "Objects must be sorted ascending by x coordinate"
        );
        Clock::wait(MODE_2_DOTS).await;

        // Mode 3 - draw pixels
        // https://gbdev.io/pandocs/Rendering.html#mode-3-length
        self.set_mode(PpuMode::Drawing);
        let mode_3_start = Clock::elapsed();
        Clock::wait(Cycles(12)).await; // Initial delay
        let y = scanline.0;
        for x in 0..SCREEN_WIDTH {
            // TODO simulate pixel FIFO
            // https://gbdev.io/pandocs/pixel_fifo.html
            let color_index = self.get_pixel(&objects, x, y);
            screen.set(x.into(), y.into(), self.get_color(color_index));
            // TODO include penalty waits
            Clock::wait(Cycles(1)).await;
        }
        // Mode 3 has a dynamic length. Whatever budget it doesn't use gets
        // rolled over to mode 0.
        let mode_3_length = Clock::elapsed() - mode_3_start;

        // Mode 0 - horizontal blank
        self.set_mode(PpuMode::HorizontalBlank);
        Clock::wait(Cycles(376) - mode_3_length).await;
    }

    /// Calculate the color index for a specific pixel
    fn get_pixel(&self, objects: &[Object], x: u8, y: u8) -> ColorIndex {
        // https://gbdev.io/pandocs/OAM.html#drawing-priority
        // First, check for objects. These are pre-sorted by x
        if let Some((tile_index, x, y)) =
            objects.iter().find_map(|object| object.get_pixel(x, y))
        {
            let tile = self.get_tile(tile_index);
            return tile.color_index(x, y);
        }

        // TODO check window
        // TODO check background
        ColorIndex::Zero
    }

    /// Look up a color from the active color palette
    fn get_color(&self, index: ColorIndex) -> Color {
        Color::new(255, 0, 0) // TODO
    }

    /// Get a list of **up to 10** visible objects for the current scanline
    ///
    /// When there are more than 10 objects intersecting with the current
    /// scanline, the objects earlier in memory (with lower addresses) get
    /// priority.
    ///
    /// Returned objects will always be sorted by x coordinate (ascending).
    ///
    /// https://gbdev.io/pandocs/OAM.html#selection-priority
    fn get_objects(&self) -> Vec<Object> {
        let line = self.registers.ly;
        // TODO the height should be changeable between objects? maybe we need
        // to delay between each object fetch
        let height = self.registers.lcdc.unpack().object_size.height();
        // Take the first 10 objects intersecting the current line
        let mut objects = self
            .oam
            .as_values()
            .iter()
            .copied()
            .map(|attributes| Object { attributes, height })
            .filter(|object| object.intersects_line(line))
            .take(10)
            .collect::<Vec<_>>();
        // Sort by x because that's what we need for render order
        objects.sort_by_key(|object| object.attributes.x);
        objects
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

    /// Set the `ppu_mode` flag of the `STAT` register
    fn set_mode(&mut self, mode: PpuMode) {
        self.registers.stat.update(|stat| LcdStatus {
            ppu_mode: mode,
            ..stat
        });
    }
}

impl Default for Gpu {
    fn default() -> Self {
        Self {
            registers: Registers::default(),
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

/// Draw mode within the current frame
///
/// This defines what the PPU is doing within a single frame draw.
/// https://gbdev.io/pandocs/Rendering.html#ppu-modes
#[derive(Debug)]
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
    /// Mode 2 - search for objects intersecting the current scanline
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

/// TODO
enum ColorIndex {
    Zero,
    One,
    Two,
    Three,
}

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

impl Tile {
    /// Get a color index for a single pixel in the tile
    ///
    /// `x` and `y` must both be in the range `[0, 7]`. This will panic
    /// otherwise.
    fn color_index(&self, x: u8, y: u8) -> ColorIndex {
        debug_assert!(
            x < 8 && y < 8,
            "Tile coordinates must be [0,7], but got ({x}, {y})"
        );
        let line = self.lines[y as usize];
        // Grab the bit corresponding to this pixel from each byte
        let bit = Bit(x);
        match (bit.get(line.low), bit.get(line.high)) {
            (false, false) => ColorIndex::Zero,
            (false, true) => ColorIndex::One,
            (true, false) => ColorIndex::Two,
            (true, true) => ColorIndex::Three,
        }
    }
}

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

impl TileIndex {
    /// Get the index of the tile after this one
    ///
    /// This is used for 8x16 tiles. The bottom tile is always the one
    /// immediately after the top tile.
    fn next(self) -> Self {
        debug_assert!(self.0 < 255, "Cannot get next tile for tile index 255");
        Self(self.0 + 1)
    }
}

/// An object that's been loaded in mode 2 and is ready to be drawn
struct Object {
    /// Attributes loaded from OAM
    attributes: ObjectAttributes,
    /// Height of the object (8 or 16), loaded from the `LCDC` register while
    /// the object is being loaded
    height: u8,
}

impl Object {
    /// Does this object intersect with the current horizontal line?
    ///
    /// The object height (8 vs 16 pixels) is determined by the `LCDC` register,
    /// so it must be passed in. This *only* checks for vertical intersection.
    /// If an object intersects vertically but is off the screen horizontally,
    /// this will **still return true.** That's consistent with the [object
    /// selection priority algorithm](https://gbdev.io/pandocs/OAM.html#selection-priority).
    fn intersects_line(&self, line: Scanline) -> bool {
        // attributes.y is shifted +16. Shift the line up to match. Subtracting
        // could incur underflow. Addition can't overflow because the max line
        // value is 153.
        let line = line.0 + 16;
        let top = self.attributes.y; // Top edge (inclusive)
        let bottom = self.attributes.y + self.height; // Bottom edge (exclusive)
        bottom > line && top <= line
    }

    /// TODO
    fn get_pixel(&self, x: u8, y: u8) -> Option<(TileIndex, u8, u8)> {
        let x = x + 8;
        let y = y + 16;
        if self.attributes.x <= x
            && x < (self.attributes.x + 8)
            && self.attributes.y <= y
            && y < (self.attributes.y + 16)
        {
            let tile_index = if y < 8 {
                self.attributes.tile_index
            } else {
                self.attributes.tile_index.next()
            };
            // Safety: these won't underflow/overflow because of the bounds
            // checks above
            let x = x - self.attributes.x;
            let y = y - self.attributes.y;
            Some((tile_index, x, y))
        } else {
            None
        }
    }
}

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
const _: () = assert!(mem::size_of::<ObjectAttributes>() == 4);

/// Flags in byte 3 of [ObjectAttributes]
///
/// This is packed as a single byte in memory; this struct is the unpacked
/// semantic data.
///
/// https://gbdev.io/pandocs/OAM.html#byte-3--attributesflags
struct ObjectFlags {
    priority: bool,
    y_flip: bool,
    x_flip: bool,
    dmg_palette: DmgPalette,
    bank: VramBank,
    cgb_palette: CgbPalette,
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
