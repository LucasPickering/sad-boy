//! Tiles and tile maps
//!
//! A [Tile] is an 8x8 array of pixels. A [TileMap] is a layer of indirection,
//! containing [TileIndex]es. These indexes map to actual tiles.

use crate::{
    emu::{
        gpu::ColorIndex,
        memory::{self, RawBytes},
    },
    util::{Bit, assert_size, assert_size_range, impl_bit_pack},
};

/// An 8x8 collection of pixels
///
/// A tile is 16 bytes:
/// - 4 colors per pixel => 2 bits per pixel
/// - 8 pixels per line => 2 bytes per line
/// - 8 lines => 16 bytes total
///
/// https://gbdev.io/pandocs/Tile_Data.html
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)] // Memory layout matters here
pub struct Tile {
    lines: [TileLine; 8],
}
assert_size!(Tile, 16);

impl Tile {
    /// Width of a tile, in pixels
    pub const WIDTH: u8 = 8;
    /// Height of a tile, in pixels
    pub const HEIGHT: u8 = 8;

    /// Create a tile from an 8x8 array of [ColorIndex]es
    #[cfg(test)]
    pub fn from_pixels(pixels: [[ColorIndex; 8]; 8]) -> Self {
        Self {
            lines: pixels.map(TileLine::from_pixels),
        }
    }

    /// Create a tile with all pixels as a single color
    #[cfg(test)]
    pub fn from_color(color: ColorIndex) -> Self {
        Self::from_pixels([[color; 8]; 8])
    }

    /// Get a color index for a single pixel in the tile
    ///
    /// `x` and `y` must both be in the range `[0, 7]`. This will panic
    /// otherwise.
    pub fn pixel(&self, x: u8, y: u8) -> ColorIndex {
        debug_assert!(
            x < 8 && y < 8,
            "Tile coordinates must be [0,7], but got ({x}, {y})"
        );
        let line = self.lines[y as usize];
        // Grab the bit corresponding to this pixel from each byte
        let bit = TileLine::bit(x);
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
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
struct TileLine {
    low: u8,
    high: u8,
}
assert_size!(TileLine, 2);

impl TileLine {
    /// Create a tile line from 8 [ColorIndex]es
    #[cfg(test)]
    fn from_pixels(pixels: [ColorIndex; 8]) -> Self {
        // There's definitely a fun bit-twiddly way to do this but I'm taking
        // the easy way for now
        let mut low = 0;
        let mut high = 0;
        for x in 0..8u8 {
            let bit = Self::bit(x);
            let (low_bit, high_bit) = match pixels[x as usize] {
                ColorIndex::Zero => (false, false),
                ColorIndex::One => (false, true),
                ColorIndex::Two => (true, false),
                ColorIndex::Three => (true, true),
            };
            low = bit.set(low, low_bit);
            high = bit.set(high, high_bit);
        }
        Self { low, high }
    }

    /// Get the bit corresponding to an x value `0-7`
    fn bit(x: u8) -> Bit {
        Bit(7 - x) // Bit 7 is x=0 (leftmost)
    }
}

/// Index of a single tile in a tile map
///
/// https://gbdev.io/pandocs/Tile_Maps.html#tile-indexes
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct TileIndex(u8);
assert_size!(TileIndex, 1);

impl TileIndex {
    #[cfg(test)]
    pub fn new(index: u8) -> Self {
        Self(index)
    }

    /// Get the index of the tile after this one
    ///
    /// This is used for 8x16 tiles. The bottom tile is always the one
    /// immediately after the top tile.
    pub fn next(self) -> Self {
        debug_assert!(self.0 < 255, "Cannot get next tile for tile index 255");
        Self(self.0 + 1)
    }
}

/// Selector for a block of tile *map* memory
///
/// Used for multiple flags in [LcdControl].
#[derive(Clone, Copy, Debug)]
pub enum TileMapArea {
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
/// At any given time two blocks are accessible: 0-1 or 2-1. In [Self::High],
/// indexes `0-127` map to the *upper* block.
/// https://gbdev.io/pandocs/Tile_Data.html
#[derive(Clone, Copy, Debug)]
pub enum TileDataArea {
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

/// An array of tile *definitions*
///
/// This is where the tile pixel data is defined. Objects reference this
/// directly, but the window and background go through [TileMap].
///
/// https://gbdev.io/pandocs/Tile_Data.html
#[derive(Clone, Debug)]
#[repr(C)]
pub struct TileData {
    /// Tile pixel data
    ///
    /// This is split into 3 logical blocks, each 128 tiles (2048 bytes).
    /// At any given time, two blocks are accessible (0-1 or 2-1) based on
    /// bit 4 of the `LCDC` register. In the latter mode, the lower indexes
    /// map to the *higher* block. See [TileDataArea] for more.
    ///
    /// https://gbdev.io/pandocs/Tile_Data.html
    data: [[Tile; Self::BLOCK_LENGTH]; 3],
}
assert_size_range!(TileData, memory::TILE_DATA);

impl TileData {
    /// Number of tiles in each block
    const BLOCK_LENGTH: usize = 128;

    /// Get a tile by index
    ///
    /// `area` defines which tile blocks are active: low+middle or middle+high.
    /// Background and window use the `bg_window_area` flag in the `LCDC`
    /// register, but objects always use [TileDataArea::Low].
    pub fn get(&self, area: TileDataArea, index: TileIndex) -> Tile {
        // SAFETY: Returned index is valid by the contract of index()
        let index = Self::index(area, index);
        self.data.as_flattened()[index]
    }

    /// Set a tile by index
    ///
    /// This is for generating test data only. The Game Boy never needs to do
    /// this; mutations are made via the memory view.
    #[cfg(test)]
    pub fn set(&mut self, area: TileDataArea, index: TileIndex, tile: Tile) {
        // SAFETY: Returned index is valid by the contract of index()
        let index = Self::index(area, index);
        self.data.as_flattened_mut()[index] = tile;
    }

    /// Get an index into `self.data` for the given area/tile index
    ///
    /// The returned index will always be valid for `self.data.as_flattened()`
    /// (`< 128*3`)
    fn index(area: TileDataArea, index: TileIndex) -> usize {
        const BL: usize = TileData::BLOCK_LENGTH;

        // Block 1 is always the UPPER. Lower can be 0 or 2
        // https://gbdev.io/pandocs/Tile_Data.html
        match area {
            TileDataArea::Low => index.0 as usize,
            // 0-127 maps to 256-383; 128-255 stays the same. You could do this
            // with a comparison, but this is BRANCHLESS!!
            TileDataArea::High => (index.0 as usize + BL) % (BL * 2) + BL,
        }
    }
}

impl Default for TileData {
    fn default() -> Self {
        Self {
            data: [[Tile::default(); Self::BLOCK_LENGTH]; 3],
        }
    }
}

// This representation matches what's used in memory
impl RawBytes for TileData {}

/// Container for two tile maps (low and high)
///
/// A tile map is a collection of tile *indexes*. Those indexes point to
/// [TileData], where the pixel values are actually stored. [TileMapArea]
/// selects between the two maps.
///
/// https://gbdev.io/pandocs/Tile_Maps.html
#[derive(Clone, Debug)]
#[repr(C)]
pub struct TileMaps {
    /// `[lower, upper]` tile maps
    maps: [[TileIndex; Self::LENGTH]; 2],
}
assert_size_range!(TileMaps, memory::TILE_MAPS);

impl TileMaps {
    /// Width of a single map, in tiles
    const WIDTH: usize = 32;
    /// Height of a single map, in tiles
    const HEIGHT: usize = 32;
    /// Number of total tiles in a single map
    const LENGTH: usize = Self::WIDTH * Self::HEIGHT;

    /// Get a tile by its coordinate
    ///
    /// `(0,0)` is the first tile, `(1,0)` is the second, etc. `area` is
    /// determined from the `LCDC` register; the exact bit to use depends on
    /// the context.
    ///
    /// https://gbdev.io/pandocs/pixel_fifo.html#get-tile
    pub fn get(&self, x: u8, y: u8, area: TileMapArea) -> TileIndex {
        let map = match area {
            TileMapArea::Low => &self.maps[0],
            TileMapArea::High => &self.maps[1],
        };
        let index = usize::from(y) * 32 + usize::from(x);
        map[index]
    }
}

impl Default for TileMaps {
    fn default() -> Self {
        Self {
            maps: [[TileIndex(0); Self::LENGTH]; 2],
        }
    }
}

// This representation matches what's used in memory
impl RawBytes for TileMaps {}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    /// Test getting tile data by index
    ///
    /// Most tiles will be initialized with default values, but one tile (#77)
    /// in each area gets a unique layout.
    #[rstest]
    #[case::low_block_0(
        TileDataArea::Low,
        77,
        Tile::from_color(ColorIndex::One)
    )]
    #[case::low_block_1(
        TileDataArea::Low,
        128 + 77,
        Tile::from_color(ColorIndex::Two)
    )]
    #[case::high_block_1(
        TileDataArea::High,
        128 + 77,
        Tile::from_color(ColorIndex::Two)
    )]
    // Low indexes are mapped to the highest block
    #[case::high_block_2(
        TileDataArea::High,
        77,
        Tile::from_color(ColorIndex::Three)
    )]
    fn tile_data_get(
        #[case] area: TileDataArea,
        #[case] index: u8,
        #[case] expected: Tile,
    ) {
        let mut tile_data = TileData::default();
        tile_data.data[0][77] = Tile::from_color(ColorIndex::One);
        tile_data.data[1][77] = Tile::from_color(ColorIndex::Two);
        tile_data.data[2][77] = Tile::from_color(ColorIndex::Three);
        let tile = tile_data.get(area, TileIndex(index));
        assert_eq!(tile, expected);
    }
}
