//! Tiles and tile maps
//!
//! A [Tile] is an 8x8 array of pixels. A [TileMap] is a layer of indirection,
//! containing [TileIndex]es. These indexes map to actual tiles.

use crate::{
    emu::{gpu::ColorIndex, memory::RawBytes},
    util::{Bit, impl_bit_pack},
};
use std::mem;

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
        let bit = Bit(x);
        match (bit.get(line.low), bit.get(line.high)) {
            (false, false) => ColorIndex::Zero,
            (false, true) => ColorIndex::One,
            (true, false) => ColorIndex::Two,
            (true, true) => ColorIndex::Three,
        }
    }
}

// Memory bus accesses this as raw bytes
impl RawBytes for Tile {}

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

impl TileLine {
    /// Create a tile line from 8 [ColorIndex]es
    #[cfg(test)]
    fn from_pixels(pixels: [ColorIndex; 8]) -> Self {
        // There's definitely a fun bit-twiddly way to do this but I'm taking
        // the easy way for now
        let mut low = 0;
        let mut high = 0;
        for x in 0..8u8 {
            let bit = Bit(x);
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
}

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
    pub fn next(self) -> Self {
        debug_assert!(self.0 < 255, "Cannot get next tile for tile index 255");
        Self(self.0 + 1)
    }
}

// Memory bus accesses this as raw bytes
impl RawBytes for TileIndex {}

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
/// At any given time two blocks are accessible: 0-1 or 1-2.
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
#[derive(Debug)]
pub struct TileData {
    /// Tile pixel data
    ///
    /// This is split into 3 logical blocks, each 128 tiles (2048 bytes).
    /// At any given time, two blocks are accessible (0-1 or 1-2) based on
    /// bit 4 of the `LCDC` register. See [TileDataArea] for more.
    ///
    /// https://gbdev.io/pandocs/Tile_Data.html
    data: [Tile; Self::BLOCK_LENGTH * 3],
}

impl TileData {
    /// Number of tiles in each block
    const BLOCK_LENGTH: usize = 128;

    /// Get a slice of **all** tiles (not just the active blocks)
    ///
    /// Use this for the memory bus.
    pub fn as_slice(&self) -> &[Tile] {
        &self.data
    }

    /// Get a mutable slice of **all** tiles (not just the active blocks)
    ///
    /// Use this for the memory bus.
    pub fn as_slice_mut(&mut self) -> &mut [Tile] {
        &mut self.data
    }

    /// Get a tile by index
    ///
    /// `area` defines which tile blocks are active: low+middle or middle+high.
    /// Background and window use the `bg_window_area` flag in the `LCDC`
    /// register, but objects always use [TileDataArea::Low].
    pub fn get(&self, index: TileIndex, area: TileDataArea) -> Tile {
        // Select tile slice based on the area
        let slice = match area {
            // Tile memory is 3 blocks of 128 tiles
            TileDataArea::Low => &self.data[..(Self::BLOCK_LENGTH * 2)],
            TileDataArea::High => &self.data[Self::BLOCK_LENGTH..],
        };
        debug_assert_eq!(slice.len(), 256, "Tile data should be 256 tiles");

        // SAFETY: Length is always 256, covered by assertion
        slice[index.0 as usize]
    }

    /// Set a tile by index
    ///
    /// This is for generating test data only. The Game Boy never needs to do
    /// this; mutations are made via the memory view.
    #[cfg(test)]
    pub fn set(&mut self, index: u8, tile: Tile) {
        // Right now only the lower 2 blocks are accessible because of the
        // bounds of u8. I'll expand that if there's a need for it.
        self.data[index as usize] = tile;
    }
}

impl Default for TileData {
    fn default() -> Self {
        Self {
            data: [Tile::default(); Self::BLOCK_LENGTH * 3],
        }
    }
}

/// Container for two tile maps (low and high)
///
/// A tile map is a collection of tile *indexes*. Those indexes point to
/// [TileData], where the pixel values are actually stored. [TileMapArea]
/// selects between the two maps.
///
/// https://gbdev.io/pandocs/Tile_Maps.html
#[derive(Debug)]
pub struct TileMaps {
    /// `[lower, upper]` tile maps
    maps: [[TileIndex; Self::LENGTH]; 2],
}

impl TileMaps {
    /// Width of a single map, in tiles
    const WIDTH: usize = 32;
    /// Height of a single map, in tiles
    const HEIGHT: usize = 32;
    /// Number of total tiles in a single map
    const LENGTH: usize = Self::WIDTH * Self::HEIGHT;

    /// Get a slice of both tile maps
    ///
    /// Use this for the memory bus.
    pub fn as_slice(&self) -> &[TileIndex] {
        self.maps.as_flattened()
    }

    /// Get a mutable slice of both tile maps
    ///
    /// Use this for the memory bus.
    pub fn as_slice_mut(&mut self) -> &mut [TileIndex] {
        self.maps.as_flattened_mut()
    }

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
