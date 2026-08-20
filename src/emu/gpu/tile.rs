//! Tiles and tile maps
//!
//! A [Tile] is an 8x8 array of pixels. A [TileMap] is a layer of indirection,
//! containing [TileIndex]es. These indexes map to actual tiles.

use crate::{emu::gpu::ColorIndex, util::Bit};
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
pub struct TileIndex(pub u8); // TODO make field private
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
