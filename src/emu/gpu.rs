//! Graphics processing
//!
//! This computes graphics output and sends it to the virtual screen. The
//! physical drawing is done in [crate::screen].

mod tile;

use crate::{
    backend::{Color, FrameBuffer},
    emu::{
        clock::{Clock, Cycles},
        gpu::tile::{
            Tile, TileData, TileDataArea, TileIndex, TileMapArea, TileMaps,
        },
        memory::{self, MemoryView},
    },
    util::{Bit, Mask, PackedBits, impl_bit_pack},
};
use std::{
    fmt::Debug,
    mem,
    ops::{Add, Sub},
};
use tracing::info_span;

/// Dots in a single scanline
const SCANLINE_DOTS: Cycles = Cycles(456);
/// Number of scanlines in each frame
const SCANLINES_PER_FRAME: u8 = 154;
/// Number of scanlines in each frame that involve drawing
///
/// This is [SCANLINES_PER_FRAME] minus the number of vertical blank lines in
/// each frame.
const SCANLINES_PER_FRAME_DRAWN: u8 = 144;

/// Graphics registers and processing
#[derive(Debug, Default)]
pub struct Gpu {
    /// Memory specific to the GPU
    vram: Vram,
    /// State machine for the current scanline being drawn
    current_scanline: ScanlineState,
}

impl Gpu {
    /// Advance the GPU one clock cycle
    ///
    /// Return `true` if this was the last tick of the current frame and the
    /// frame is now ready to be drawn.
    pub(super) fn tick(
        &mut self,
        clock: &Clock,
        frame: &mut FrameBuffer,
    ) -> bool {
        let _span = info_span!("GPU");

        // Each scanline is a fixed number of dots, so we can calculate the
        // scanline from the current clock cycle
        let (scanline, scanline_dots) = Scanline::from_clock(clock);
        let ppu_mode = match scanline.0 {
            0..SCANLINES_PER_FRAME_DRAWN => {
                // Each state is calculated from the previous and may need to
                // carry over owned values. We'll temporarily take it out of its
                // slot, then replace it right after.
                let scanline_state = mem::take(&mut self.current_scanline);
                self.current_scanline = scanline_state.tick(
                    scanline,
                    scanline_dots,
                    &self.vram,
                    frame,
                );
                // TODO should we grab mode before or after transition?
                self.current_scanline.mode()
            }
            SCANLINES_PER_FRAME_DRAWN..SCANLINES_PER_FRAME => {
                self.current_scanline = ScanlineState::Start;
                PpuMode::VerticalBlank
            }
            _ => unreachable!("Invalid scanline: {scanline:?}"),
        };

        // Update registers
        // TODO does this need to be done before the calculation? probably!!
        let reg = &mut self.vram.registers;
        reg.ly = scanline; // Update LY register
        reg.stat.update(|stat| LcdStatus {
            ppu_mode,
            lyc_equal_ly: reg.ly == reg.lyc,
            ..stat
        });

        clock.is_frame_end()
    }

    /// Get read-only access to the GPU I/O registers
    pub fn registers(&self) -> &Registers {
        &self.vram.registers
    }

    /// Get mutable access to the GPU I/O registers
    pub fn registers_mut(&mut self) -> &mut Registers {
        &mut self.vram.registers
    }

    /// Access the Object Attribute Memory
    ///
    /// OAM is only accessible in modes 0 and 1. In modes 2 and 3, reads will
    /// return 0 and writes will do nothing.
    pub fn oam(&self) -> MemoryView<'_> {
        // OAM is only accessible to the CPU in blank modes
        let range = memory::OAM;
        match self.mode() {
            PpuMode::HorizontalBlank | PpuMode::VerticalBlank => {
                MemoryView::from_slice(&self.vram.oam, range)
            }
            PpuMode::OamScan | PpuMode::Drawing => MemoryView::null(range),
        }
    }

    /// Access tile data memory
    ///
    /// VRAM is only accessible in modes 0-2. In mode 3, reads will return 0 and
    /// writes will do nothing.
    pub fn tile_data(&self) -> MemoryView<'_> {
        // VRAM is not accessible in mode 3
        let range = memory::TILE_DATA;
        match self.mode() {
            PpuMode::OamScan
            | PpuMode::HorizontalBlank
            | PpuMode::VerticalBlank => {
                MemoryView::from_slice(self.vram.tile_data.as_slice(), range)
            }
            PpuMode::Drawing => MemoryView::null(range),
        }
    }

    /// Access tile map memory
    ///
    /// VRAM is only accessible in modes 0-2. In mode 3, reads will return 0 and
    /// writes will do nothing.
    pub fn tile_maps(&self) -> MemoryView<'_> {
        // VRAM is not accessible in mode 3
        let range = memory::TILE_MAPS;
        match self.mode() {
            PpuMode::OamScan
            | PpuMode::HorizontalBlank
            | PpuMode::VerticalBlank => {
                MemoryView::from_slice(self.vram.tile_maps.as_slice(), range)
            }
            PpuMode::Drawing => MemoryView::null(range),
        }
    }

    /// Read the `ppu_mode` flag of the `STAT` register
    fn mode(&self) -> PpuMode {
        self.vram.registers.stat.unpack().ppu_mode
    }
}

#[cfg(test)]
impl Gpu {
    /// Tick until the next frame is done
    fn draw_frame(&mut self, clock: &mut Clock, frame: &mut FrameBuffer) {
        loop {
            // Clock has to tick first
            clock.tick();
            if self.tick(clock, frame) {
                break;
            }
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
    /// Background scroll X
    pub scx: u8,
    /// Background scroll Y
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
    /// Enable/disable the window
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
    /// Enable/disable object display
    object_enable: bool,
    /// Enable/disable the background AND window
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

/// Size of the next object to draw (flag in [LcdControl])
#[derive(Clone, Copy, Debug, PartialEq)]
enum ObjectSize {
    /// 8x8
    Small,
    /// 8x16
    Large,
}

impl ObjectSize {
    fn height(self) -> u8 {
        match self {
            ObjectSize::Small => Tile::HEIGHT,
            ObjectSize::Large => Tile::HEIGHT * 2,
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
///
/// https://gbdev.io/pandocs/Interrupt_Sources.html#int-48--stat-interrupt
#[derive(Debug)]
pub struct LcdStatus {
    /// Enable the `LY == LYC` condition for the `STAT` interrupt
    lyc_interrupt: bool,
    /// Enable the Mode 2 condition for the `STAT` interrupt
    mode_2_interrupt: bool,
    /// Enable the Mode 1 condition for the `STAT` interrupt
    mode_1_interrupt: bool,
    /// Enable the Mode 0 condition for the `STAT` interrupt
    mode_0_interrupt: bool,
    /// Is the `LY` register currently equal to the `LYC` register?
    ///
    /// See [Registers] for those register definitions.
    lyc_equal_ly: bool,
    /// Phase of the frame currently being drawn
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

/// Graphics-related memory
#[derive(Debug)]
pub struct Vram {
    /// 1-byte control registers related to graphics processing
    registers: Registers,
    /// Object Attribute Memory
    ///
    /// This is a list of up to 40 moveable objects.
    ///
    /// https://gbdev.io/pandocs/OAM.html
    oam: [ObjectAttributes; 40],
    /// Pixel data for tiles
    ///
    /// https://gbdev.io/pandocs/Tile_Data.html
    tile_data: TileData,
    /// Two 32x32 tile maps (lower and upper)
    ///
    /// https://gbdev.io/pandocs/Tile_Maps.html
    tile_maps: TileMaps,
}

impl Vram {
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
        let height = self.lcdc().object_size.height();
        // Take the first 10 objects intersecting the current line
        let mut objects = self
            .oam
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

    /// Calculate the color index for a specific pixel
    ///
    /// This is the main rendering logic that walks through
    /// objects/window/background.
    fn get_pixel(&self, objects: &[Object], x: u8, y: u8) -> ColorIndex {
        // https://gbdev.io/pandocs/OAM.html#drawing-priority

        self.get_object_pixel(objects, x, y)
            // TODO check window
            .unwrap_or_else(|| self.get_background_pixel(x, y))
    }

    /// Calculate a pixel from visible objects
    ///
    /// Return `None` if no objects intersect the pixel.
    fn get_object_pixel(
        &self,
        objects: &[Object],
        x: u8,
        y: u8,
    ) -> Option<ColorIndex> {
        let lcdc = self.lcdc();
        if lcdc.object_enable {
            // These are pre-sorted by x
            if let Some((tile_index, x, y)) = objects
                .iter()
                .find_map(|object| object.get_pixel(x, y, lcdc.object_size))
            {
                let tile = self.tile_data.get(tile_index, TileDataArea::Low);
                return Some(tile.pixel(x, y));
            }
        }
        None
    }

    /// Calculate a pixel from the background map
    ///
    /// The background covers the entire screen and wraps at the edge, so every
    /// pixel will have a background color.
    fn get_background_pixel(&self, x: u8, y: u8) -> ColorIndex {
        // https://gbdev.io/pandocs/Scrolling.html#ff42ff43--scy-scx-background-viewport-y-position-x-position
        // Map the x/y coordinate within the tile map. This will scroll and
        // intentionally wraps at the end of the map boundary. The map is 32x32
        // tiles and each tile is 8x8, so it's 256x256 pixels.
        let x = self.registers.scx.wrapping_add(x);
        let y = self.registers.scy.wrapping_add(y);

        // First we need to find the tile INDEX in the tile MAP, then use THAT
        // index to find the underlying TILE
        // TODO use a const for tile map width. Maybe TileMap should be a
        // struct?
        let tile_x = x / Tile::WIDTH;
        let tile_y = y / Tile::HEIGHT;
        // TODO select tile map correctly
        // https://gbdev.io/pandocs/pixel_fifo.html#get-tile
        let tile_index = self.tile_maps.get(tile_x, tile_y, TileMapArea::Low);

        // Now convert the index to an actual tile
        let tile = self.tile_data.get(tile_index, self.lcdc().bg_window_tiles);
        // Get the pixel coordinates within the tile
        tile.pixel(x % Tile::WIDTH, y % Tile::HEIGHT)

        // TODO the scroll registers should only be changeable on each tile
        // fetch (or at the beginning of the scanline), not on each pixel. The
        // entire rendering pipeline needs a rewrite to model the FIFO.
        // TODO how do we return WHITE if disabled in LCDC? it may not be in the
        // palette
    }

    /// Look up a color from the active color palette
    ///
    /// https://gbdev.io/pandocs/Palettes.html
    fn get_color(&self, index: ColorIndex) -> Color {
        // TODO look this up in the BGP register
        match index {
            ColorIndex::Zero => Color::BLACK,
            ColorIndex::One => Color::DARK_GRAY,
            ColorIndex::Two => Color::LIGHT_GRAY,
            ColorIndex::Three => Color::WHITE,
        }
    }

    /// Get the unpacked value of the `LCDC` register
    fn lcdc(&self) -> LcdControl {
        // It may be slow to repeatedly unpack this all the time. Maybe we could
        // cache it? Or provide some way to decode a single bit a time?
        self.registers.lcdc.unpack()
    }
}

impl Default for Vram {
    fn default() -> Self {
        Self {
            registers: Registers::default(),
            oam: [ObjectAttributes::default(); 40],
            tile_data: TileData::default(),
            tile_maps: TileMaps::default(),
        }
    }
}

/// Draw state of a single scanline
///
/// This is a state machine that progresses as the scanline is drawn. This is
/// *not* used for vblank scanlines, since those are stateless. See [PpuMode].
#[derive(Debug, Default)]
enum ScanlineState {
    /// Initial state: no work has been done
    #[default]
    Start,
    /// Objects have been scanned
    OamScan { objects: Vec<Object> },
    /// Pixels are being drawn to the screen
    Drawing { objects: Vec<Object>, x: u8 },
    /// Scanline is done, waiting for the next line
    HorizontalBlank,
}

impl ScanlineState {
    fn mode(&self) -> PpuMode {
        match self {
            Self::Start | Self::OamScan { .. } => PpuMode::OamScan,
            Self::Drawing { .. } => PpuMode::Drawing,
            Self::HorizontalBlank => PpuMode::HorizontalBlank,
        }
    }

    /// Advance scanline drawing one cycle
    ///
    /// ## Params
    ///
    /// - `scanline`: y value of the scanline being drawn
    /// - `dots`: number of elapsed dots in this scanline so far (first call is
    ///   0)
    fn tick(
        self,
        scanline: Scanline,
        dots: Cycles,
        vram: &Vram,
        frame: &mut FrameBuffer,
    ) -> Self {
        const SCREEN_WIDTH: u8 = FrameBuffer::WIDTH as u8;
        /// Length of mode 2 (OAM scan)
        const OAM_DURATION: Cycles = Cycles(80);
        /// Delay in mode 3 (draw) before drawing starts
        const DRAW_DELAY: Cycles = Cycles(12);

        let next_dot = dots + 1;

        // Mode numbers are not sequential by the order they occur. They're
        // numbered based on how they're represented in the STAT register
        match self {
            // Mode 2 - OAM scan
            Self::Start => {
                // I didn't find anything in the docs about the actual rate
                // that the GB collects objects per dot, so I'm doing it all
                // up front. This may have a semantic impact, I'm not sure.
                let objects = vram.get_objects();
                // Render order relies on the objects being sorted
                // NOTE: This is only for non-CGB mode. In CGB mode this
                // will have to change
                debug_assert!(
                    objects.is_sorted_by_key(|object| object.attributes.x),
                    "Objects must be sorted ascending by x coordinate"
                );
                Self::OamScan { objects }
            }
            // Stay in mode 2
            state @ Self::OamScan { .. } if next_dot < OAM_DURATION => state,
            // End of OAM scan - transition to mode 3
            Self::OamScan { objects } => Self::Drawing { objects, x: 0 },
            // Mode 3 - draw pixels
            // https://gbdev.io/pandocs/Rendering.html#mode-3-length
            // TODO this length is supposed to be dynamic - include penalties
            Self::Drawing {
                objects,
                x: x @ 0..SCREEN_WIDTH,
            } => {
                let elapsed = dots - OAM_DURATION;

                // There's an initial 12-cycle delay per line
                if elapsed < DRAW_DELAY {
                    Self::Drawing { objects, x }
                } else {
                    // Calculate the color for a single pixel
                    let y = scanline.0;
                    // TODO simulate pixel FIFO
                    // https://gbdev.io/pandocs/pixel_fifo.html
                    let color_index = vram.get_pixel(&objects, x, y);
                    frame.set(x.into(), y.into(), vram.get_color(color_index));
                    Self::Drawing { objects, x: x + 1 }
                }
            }
            // Hit the end of the line - transition to mode 0
            Self::Drawing {
                x: SCREEN_WIDTH.., ..
            } => Self::HorizontalBlank,
            // Mode 0 - horizontal blank
            // Stay in this mode until the end of the scanline
            Self::HorizontalBlank if next_dot < SCANLINE_DOTS => {
                ScanlineState::HorizontalBlank
            }
            // Reset for the next line
            Self::HorizontalBlank => ScanlineState::Start,
        }
    }
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
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct Scanline(u8);

impl Scanline {
    /// Based on the current clock tick, get the current scanline and dot within
    /// that scanline
    fn from_clock(clock: &Clock) -> (Self, Cycles) {
        let frame_dots = clock.cycles().0 % Clock::CYCLES_PER_FRAME.0;
        let scanline = frame_dots / SCANLINE_DOTS.0;
        debug_assert!(scanline <= SCANLINES_PER_FRAME.into());
        let dots = Cycles(frame_dots % SCANLINE_DOTS.0);
        debug_assert!(dots < SCANLINE_DOTS);

        // Cast is safe because of the assertions
        (Self(scanline as u8), dots)
    }
}

impl From<u8> for Scanline {
    fn from(value: u8) -> Self {
        Self(value)
    }
}

impl From<Scanline> for u8 {
    fn from(value: Scanline) -> Self {
        value.0
    }
}

/// Index of a color within the active palette
///
/// https://gbdev.io/pandocs/Palettes.html
#[derive(Clone, Copy, Debug, PartialEq)]
enum ColorIndex {
    Zero,
    One,
    Two,
    Three,
}

/// An object that's been loaded in mode 2 and is ready to be drawn
#[derive(Debug)]
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
        let line = Shifted::shift(line.0);
        let top = self.attributes.y; // Top edge (inclusive)
        let bottom = self.attributes.y + self.height; // Bottom edge (exclusive)
        bottom > line && top <= line
    }

    /// Check if a pixel intersects this object
    ///
    /// `size` is the current object size from the `LCDC` register.
    ///
    /// Return the tile that the pixel should be grabbed from, as well as the
    /// `(x, y)` offset into that tile. Return `None` if the pixel doesn't
    /// intersect with this object.
    fn get_pixel(
        &self,
        x: u8,
        y: u8,
        size: ObjectSize,
    ) -> Option<(TileIndex, u8, u8)> {
        // attributes.x/y are shifted; shift the input coordinates to match
        let x = Shifted::shift(x);
        let y = Shifted::shift(y);
        if self.attributes.x <= x
            && x < (self.attributes.x + Tile::WIDTH)
            && self.attributes.y <= y
            && y < (self.attributes.y + size.height())
        {
            // Shift x/y to be relative to the tile start
            // SAFETY: these won't underflow/overflow because of the bounds
            // checks above
            let x = x - self.attributes.x;
            let y = y - self.attributes.y;

            // Bounds checking enforces these
            debug_assert!(x < 8);
            debug_assert!(y < 16);

            // For large objects, check if this is the upper or lower tile
            let tile_index = if size == ObjectSize::Large && y >= 8 {
                self.attributes.tile_index.next()
            } else {
                self.attributes.tile_index
            };

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
struct ObjectAttributes {
    // Field order must match the doc above
    /// Vertical position of the object + 16
    ///
    /// The +16 allows moving an object above the screen without underflowing
    /// the byte.
    y: Shifted<16>,
    /// Horizontal position of the object + 8
    ///
    /// The +8 allows moving an object left of the screen without underflowing
    /// the byte.
    x: Shifted<8>,
    /// Index of the tile defining this object
    ///
    /// For 8x8 tiles, this is the index into the tile map for the object's
    /// only tile. For 8x16 tiles, it's the index of the first (upper) tile,
    /// and the lower tile is the subsequent tile in the map.
    tile_index: TileIndex,
    /// Additional object metadata affecting its presentation
    flags: PackedBits<ObjectFlags>,
}
const _: () = assert!(mem::size_of::<ObjectAttributes>() == 4);

/// An x/y value increased by a static amount
///
/// The contained value *has been shifted* by adding `N` to it. To get the
/// original value back, subtract `N`.
///
/// This is a newtype wrapper for [ObjectAttributes::x] and
/// [ObjectAttributes::y]. These fields are shifted up statically to allow for
/// "negative" off-screen coordinates within a signed value. The newtype makes
/// it harder to write buggy code related to these fields.
///
/// The const parameter ensures the 1-byte runtime size.
///
/// **This should never be constructed directly.** Use [Shifted::default] or
/// [Shifted::shift].
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
struct Shifted<const N: u8>(u8);

impl<const N: u8> Shifted<N> {
    /// Create a new [Shifted] containing `value + N`
    fn shift(value: u8) -> Self {
        Self(value + N)
    }
}

/// Addition with `u8` retains the `Shifted` wrapper
impl<const N: u8> Add<u8> for Shifted<N> {
    type Output = Self;

    fn add(self, rhs: u8) -> Self::Output {
        Self(self.0 + rhs)
    }
}

/// Subtraction between two `Shifted`s cancels out the shift, so it returns a
/// `u8`
impl<const N: u8> Sub for Shifted<N> {
    type Output = u8;

    fn sub(self, rhs: Self) -> Self::Output {
        self.0 - rhs.0
    }
}

/// Flags in byte 3 of [ObjectAttributes]
///
/// This is packed as a single byte in memory; this struct is the unpacked
/// semantic data.
///
/// https://gbdev.io/pandocs/OAM.html#byte-3--attributesflags
#[derive(Default)]
struct ObjectFlags {
    /// Control object transparency
    priority: ObjectPriority,
    /// Flip the object vertically?
    y_flip: bool,
    /// Flip the object horizontally?
    x_flip: bool,
    /// Color palette selection for DMG (original Game Boy) mode
    dmg_palette: DmgPalette,
    /// Which swappable VRAM bank is loaded?
    bank: VramBank,
    /// Color palette selection for CGB (Game Boy Color) mode
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

/// Control object transparency
///
/// This controls how color index 0 is handled in objects. It can be used to
/// make objects transparent, such that they render behind the window and
/// background.
#[derive(Default)]
enum ObjectPriority {
    /// Color index 0 is drawn as color 0
    #[default]
    Object,
    /// Color index 0 is transparent (background/window draw on top)
    Background,
}

impl_bit_pack! {
    enum ObjectPriority;
    0b0 => Object,
    0b1 => Background,
}

/// Color palette selection in OAM flags for DMG (original Game Boy) mode
#[derive(Default)]
enum DmgPalette {
    #[default]
    Obp0,
    Obp1,
}

impl_bit_pack! {
    enum DmgPalette;
    0b0 => Obp0,
    0b1 => Obp1,
}

/// VRAM bank selection in OAM flags
#[derive(Default)]
enum VramBank {
    #[default]
    Bank0,
    Bank1,
}

impl_bit_pack! {
    enum VramBank;
    0b0 => Bank0,
    0b1 => Bank1,
}

/// Color palette selection in OAM flags for CGB (Game Boy Color) mode
#[derive(Default)]
enum CgbPalette {
    #[default]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::BitPack;

    /// Test drawing a simple object to the screen
    #[test]
    fn objects() {
        let mut clock = Clock::new();
        let mut gpu = Gpu::default();
        let mut frame = FrameBuffer::new();

        // Create a tile of all light gray
        let vram = &mut gpu.vram;
        vram.registers.lcdc.update(|lcdc| LcdControl {
            object_enable: true,
            ..lcdc
        });
        vram.tile_data
            .set(0, Tile::from_pixels([[ColorIndex::Two; 8]; 8]));

        // Put an object in the top-left with that tile
        vram.oam[0] = ObjectAttributes {
            y: Shifted::shift(0),
            x: Shifted::shift(0),
            tile_index: TileIndex::default(),
            flags: ObjectFlags::default().pack(),
        };

        // Render one frame
        gpu.draw_frame(&mut clock, &mut frame);

        let mut expected =
            FrameBuffer::from_pixels(vec![Color::BLACK; FrameBuffer::LENGTH]);
        expected.set_region(0, 0, 8, 8, Color::LIGHT_GRAY);
        frame.assert_pixels(&expected);
    }

    /// Test [Tile::color_index]
    #[test]
    fn tile_color_index() {
        let row = [
            ColorIndex::Zero,
            ColorIndex::One,
            ColorIndex::Two,
            ColorIndex::Three,
            ColorIndex::Three,
            ColorIndex::Two,
            ColorIndex::One,
            ColorIndex::Zero,
        ];
        let lines = [row; 8];
        let tile = Tile::from_pixels(lines);

        for x in 0..8u8 {
            for y in 0..8u8 {
                let expected = lines[y as usize][x as usize];
                let actual = tile.pixel(x, y);
                assert_eq!(actual, expected, "Mismatch at ({x}, {y})");
            }
        }
    }
}
