mod memory_view;

pub use memory_view::MemoryView;

use crate::{
    emu::{
        gpu::Gpu,
        instruction::Instruction,
        rom::{self, Rom},
    },
    util::IntDisplay,
};
use std::{
    fmt::{self, Debug, Display},
    ops::{Add, AddAssign},
    range::RangeInclusive,
    str::FromStr,
};
use tracing::error;
use winnow::{Parser, error::ContextError};

/// Code executed at boot, before entering the ROM
///
/// The last instruction executed by the bootstrap will unmap itself by writing
/// to the `BANK` register (`0xFF50`).
///
/// https://gbdev.io/pandocs/Power_Up_Sequence.html
///
/// Downloaded from https://gbdev.gg8.se/files/roms/bootroms/
const BOOTSTRAP_CODE: &[u8] = include_bytes!("../../bootstrap/dmg_boot.bin");
// ===== Memory Blocks =====
// https://gbdev.io/pandocs/Memory_Map.html
/// Range containing the bootstrap code
///
/// This range is mapped *on top of* the ROM code while the bootstrap is
/// running.
pub const BOOTSTRAP: AddressRange =
    AddressRange::new("Bootstrap", 0x0000, 0x0100);
/// Static portion of the cartridge ROM
///
/// This is the first bank of the ROM, which **cannot** be switched.
pub const CARTRIDGE_ROM_0: AddressRange =
    AddressRange::new("Cartridge ROM Bank 0", 0x0000, 0x3FFF);
/// Switchable bank of the cartridge ROM
///
/// A cartridge can have any number of secondary banks and switch between them.
pub const CARTRIDGE_ROM_N: AddressRange =
    AddressRange::new("Cartridge ROM Bank N", 0x4000, 0x7FFF);
/// All read-only cartridge memory
pub const CARTRIDGE_ROM: AddressRange =
    CARTRIDGE_ROM_0.union("Cartridge ROM", CARTRIDGE_ROM_N);
/// Video RAM containing tile pixel data
pub const TILE_DATA: AddressRange =
    AddressRange::new("Tile Data", 0x8000, 0x97FF);
/// Video RAM containing both tile maps
pub const TILE_MAPS: AddressRange =
    AddressRange::new("Tile Maps", 0x9800, 0x9FFF);
/// Switchable RAM bank provided by the cartridge
pub const CARTRIDGE_RAM: AddressRange =
    AddressRange::new("Cartridge RAM", 0xA000, 0xBFFF);
/// Address range for general-purpose writable RAM
pub const RAM: AddressRange = AddressRange::new("RAM", 0xC000, 0xDFFF);
/// A mirror of RAM that *should* not be used by games
const ECHO_RAM: AddressRange = AddressRange::new("Echo RAM", 0xE000, 0xFDFF);
/// Object Attribute Memory (part of VRAM)
pub const OAM: AddressRange = AddressRange::new("OAM", 0xFE00, 0xFE9F);
/// Address range for additional general-purpose writable RAM
pub const HIGH_RAM: AddressRange =
    AddressRange::new("High RAM", 0xFF80, 0xFFFE);
// ===== Hardware Registers ====
// https://gbdev.io/pandocs/Hardware_Reg_List.html
pub const LCDC: u16 = 0xFF40;
pub const STAT: u16 = 0xFF41;
pub const SCY: u16 = 0xFF42;
pub const SCX: u16 = 0xFF43;
pub const LY: u16 = 0xFF44;
pub const LYC: u16 = 0xFF45;
pub const DMA: u16 = 0xFF46;
pub const BANK: u16 = 0xFF50;

/// Generate `x_START` and `x_END` consts for a set of memory ranges
///
/// These consts are needed to use the start/end in pattern matching, where
/// complex expressions aren't allowed.
macro_rules! bounds {
    ($($range:expr),* $(,)?) => {
        paste::paste! {
            $(
                const [<$range _START>]: u16 = $range.start().0;
                const [<$range _LAST>]: u16 = $range.last().0;
            )*
        }
    };
}

// Generate extra consts for pattern matching
bounds!(
    BOOTSTRAP,
    CARTRIDGE_ROM_0,
    CARTRIDGE_ROM_N,
    CARTRIDGE_ROM,
    TILE_DATA,
    TILE_MAPS,
    CARTRIDGE_RAM,
    RAM,
    ECHO_RAM,
    OAM,
    HIGH_RAM
);

/// An abstraction over the addessable range of memory
///
/// All parts of accessible memory are held as references. This aliases each
/// component based on given memory addresses. This allows each component of
/// memory/registers/etc. to be owned by its relevant module and handed out to
/// the CPU only as needed. This doesn't own any emulator state itself because
/// this struct is ephemeral. It's thrown away at the end of each tick
///
/// https://gbdev.io/pandocs/Memory_Map.html
#[derive(Debug)]
pub struct MemoryBus<'a> {
    /// RAM and registers
    ram: &'a mut RandomAccessMemory,
    /// Read-only memory from the cartridge
    rom: &'a Rom,
    /// VRAM and graphics-related IO registers
    gpu: &'a mut Gpu,
}

impl<'a> MemoryBus<'a> {
    /// Construct a memory bus from references to each addressable component
    pub fn new(
        memory: &'a mut RandomAccessMemory,
        rom: &'a Rom,
        gpu: &'a mut Gpu,
    ) -> Self {
        Self {
            ram: memory,
            rom,
            gpu,
        }
    }

    /// Load the CPU instruction at the given address
    ///
    /// Return the instruction as well as the number of bytes it consumed. This
    /// is the number of bytes that the PC should advance.
    pub fn get_instruction(&self, address: Address) -> (Instruction, u16) {
        // Instruction parsing is set up to read from either the bootstrap or
        // the ROM. Reading from anywhere else is a bug.
        //
        // Since parsing requires a slice instead of accessing bytes one at a
        // time, this is easier than supporting instruction parsing from
        // arbitrary addresses.
        assert!(
            CARTRIDGE_ROM.contains(address),
            "Requested instruction at {address} is out of range {CARTRIDGE_ROM}"
        );

        // Cache instructions because parsing is expensive
        let source = if self.is_bootstrapping() {
            // If the bootstrap is enabled, then we should only be running
            // bootstrap code. Out-of-bounds here implies the bootstrap exited
            // without unmapping itself, or something else re-mapped the
            // bootstrap.
            BOOTSTRAP_CODE
        } else {
            self.rom.bytes()
        };
        rom::parse_instruction(source, address).unwrap_or_else(|error| {
            panic!("Failed to parse instruction: {error}");
        })
    }

    /// Get a 1-byte value from memory
    ///
    /// All 16-bit addresses are valid, so this is infallible.
    pub fn get8(&self, address: Address) -> u8 {
        // https://gbdev.io/pandocs/Memory_Map.html
        match address.0 {
            BOOTSTRAP_START..=BOOTSTRAP_LAST if self.is_bootstrapping() => {
                let index: usize = address.0.into();
                BOOTSTRAP_CODE[index]
            }
            // Cartridge ROM
            CARTRIDGE_ROM_0_START..=CARTRIDGE_ROM_0_LAST => {
                // SAFETY: Cartridge ROM asserts its own length
                let index: usize = address.0.into();
                self.rom.bytes()[index]
            }
            CARTRIDGE_ROM_N_START..=CARTRIDGE_ROM_N_LAST => {
                error!("TODO: Game ROM bank N");
                0
            }
            TILE_DATA_START..=TILE_DATA_LAST => {
                self.gpu.tile_data().get(address)
            }
            TILE_MAPS_START..=TILE_MAPS_LAST => {
                self.gpu.tile_maps().get(address)
            }
            CARTRIDGE_RAM_START..=CARTRIDGE_RAM_LAST => {
                self.ram.cartridge_ram().get(address)
            }
            RAM_START..=RAM_LAST => self.ram.ram().get(address),
            ECHO_RAM_START..=ECHO_RAM_LAST => {
                // TODO dedupe with set8
                // Make sure mirrored references can't go out of bounds
                debug_assert!(ECHO_RAM.len() <= RAM.len());
                // Shift to the main RAM section
                let address = Address(address.0 - ECHO_RAM_START + RAM_START);
                self.get8(address)
            }
            OAM_START..=OAM_LAST => self.gpu.oam().get(address),
            0xFEA0..=0xFEFF => 0, // Null mem

            // Hardware registers
            LCDC => self.gpu.registers().lcdc.into(),
            STAT => self.gpu.registers().stat.into(),
            SCY => self.gpu.registers().scy,
            SCX => self.gpu.registers().scx,
            LY => self.gpu.registers().ly.into(),
            LYC => self.gpu.registers().lyc.into(),
            DMA => self.gpu.registers().dma,
            BANK => self.ram.bank,
            0xFF00..=0xFF7F => {
                error!("TODO: unmapped I/O register {address}");
                0
            }

            HIGH_RAM_START..=HIGH_RAM_LAST => self.ram.high_ram().get(address),
            0xFFFF => {
                error!("TODO: Interrupt Enabled Register read");
                0
            }
        }
    }

    /// Set a 1-byte value in memory
    ///
    /// If the memory isn't writable, this does nothing.
    pub fn set8(&mut self, address: Address, value: u8) {
        // https://gbdev.io/pandocs/Memory_Map.html
        match address.0 {
            // ROM is immutable (bootstrapp too, so it doesn't matter if it's
            // mapped or not)
            CARTRIDGE_ROM_START..=CARTRIDGE_ROM_LAST => {}
            TILE_DATA_START..=TILE_DATA_LAST => {
                self.gpu.tile_data().set(address, value);
            }
            TILE_MAPS_START..=TILE_MAPS_LAST => {
                self.gpu.tile_maps().set(address, value);
            }
            CARTRIDGE_RAM_START..=CARTRIDGE_RAM_LAST => {
                self.ram.cartridge_ram().set(address, value);
            }
            RAM_START..=RAM_LAST => self.ram.ram().set(address, value),
            ECHO_RAM_START..=ECHO_RAM_LAST => {
                // Make sure mirrored references can't go out of bounds
                debug_assert!(ECHO_RAM.len() <= RAM.len());
                // Shift to the main RAM section
                let address = Address(address.0 - ECHO_RAM_START + RAM_START);
                self.set8(address, value);
            }
            OAM_START..=OAM_LAST => self.gpu.oam().set(address, value),
            0xFEA0..=0xFEFF => {} // Null mem

            // Hardware registers
            LCDC => self.gpu.registers_mut().lcdc = value.into(),
            STAT => self.gpu.registers_mut().stat = value.into(),
            SCY => self.gpu.registers_mut().scy = value,
            SCX => self.gpu.registers_mut().scx = value,
            LY => self.gpu.registers_mut().ly = value.into(),
            LYC => self.gpu.registers_mut().lyc = value.into(),
            DMA => self.gpu.registers_mut().dma = value,
            BANK => self.ram.bank = value,
            0xFF00..=0xFF7F => error!("TODO: unmapped I/O register {address}"),

            HIGH_RAM_START..=HIGH_RAM_LAST => {
                self.ram.high_ram().set(address, value);
            }

            0xFFFF => error!("TODO: Interrupt Enabled Register write"),
        }
    }

    /// Get a 2-byte value from memory
    pub fn get16(&self, address: Address) -> u16 {
        let low = self.get8(address);
        let high = self.get8(address + 1);
        u16::from_le_bytes([low, high]) // Game Boy is little-endian
    }

    /// Set a 2-byte value in memory
    ///
    /// If the memory isn't writable, this does nothing.
    pub fn set16(&mut self, address: Address, value: u16) {
        // This would be more exciting with `unsafe`, but the alignment stuff
        // is annoying to deal with
        let [low, high] = value.to_le_bytes(); // Game Boy is little-endian
        self.set8(address, low);
        self.set8(address + 1, high);
    }

    /// Is the bootstrapp currently mapped?
    ///
    /// This is `true` only during initial boot. The bootstrap unmaps itself
    /// with its last instruction, at which point it should never be re-mapped.
    pub fn is_bootstrapping(&self) -> bool {
        self.ram.bank == 0
    }
}

/// Address of a byte of memory
///
/// The Game Boy memory range covers the entire `u16` range, so all addresses
/// are valid.
///
/// https://rylev.github.io/DMG-01/public/book/memory_map.html
#[derive(Clone, Copy, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Address(pub u16);

impl Add<u16> for Address {
    type Output = Self;

    fn add(self, rhs: u16) -> Self::Output {
        Self(self.0 + rhs)
    }
}

impl AddAssign<u16> for Address {
    fn add_assign(&mut self, rhs: u16) {
        self.0 += rhs;
    }
}

impl Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(&self, f) // Defer to Display
    }
}

impl Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", IntDisplay::hex(self.0))
    }
}

impl FromStr for Address {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        winnow::ascii::hex_uint::<_, u16, ContextError>
            .parse(s)
            .map(Self)
            .map_err(|e| e.to_string())
    }
}

/// A range of memory addresses
#[derive(Clone, Copy, Debug)]
pub struct AddressRange {
    name: &'static str,
    range: RangeInclusive<Address>,
}

impl AddressRange {
    /// Define a range of memory
    ///
    ///
    /// The given bounds are **inclusive**: `[start, end]`.
    pub const fn new(name: &'static str, start: u16, last: u16) -> Self {
        Self {
            name,
            range: RangeInclusive {
                start: Address(start),
                last: Address(last),
            },
        }
    }

    /// Join two contiguous ranges
    ///
    /// `self` must be the lower range and `other` is the upper range.
    const fn union(self, name: &'static str, other: AddressRange) -> Self {
        Self {
            name,
            range: RangeInclusive {
                start: self.start(),
                last: other.last(),
            },
        }
    }

    /// Get the number of bytes in the range
    pub const fn len(&self) -> usize {
        // The end is inclusive, so we need +1 to count it
        (self.range.last.0 - self.range.start.0 + 1) as usize
    }

    /// First address included in the range
    pub const fn start(&self) -> Address {
        self.range.start
    }

    /// Last address included in the range
    pub const fn last(&self) -> Address {
        self.range.last
    }

    /// Get the offset between the given address and the start of this range
    ///
    /// Panics if the address is not in this range. Use this for pointer math on
    /// addressed memory.
    pub fn offset(&self, address: Address) -> usize {
        assert!(
            self.contains(address),
            "Address {address} out of bounds {self}",
        );
        (address.0 - self.start().0).into()
    }

    /// Is the address within this range?
    pub fn contains(&self, address: Address) -> bool {
        self.range.contains(&address)
    }
}

impl Display for AddressRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let range = &self.range;
        write!(f, "{} [{}, {}]", self.name, range.start, range.last)
    }
}

/// Container for RAM and memory-related registers
#[derive(Debug)]
pub struct RandomAccessMemory {
    // ===== RAM =====
    /// General-purpose writable memory
    ///
    /// This is boxed because 8KiB is too big to reasonably put on the stack.
    ram: [u8; RAM.len()],
    /// Additional general-purpose writable memory
    ///
    /// This is most commonly used when accessed by the `LD HL, SP+imm8`
    /// instruction.
    high_ram: [u8; HIGH_RAM.len()],
    /// Additional RAM provided by the cartridge
    ///
    /// This is similar to the other two RAM blocks, except some cartridges can
    /// provide multiple banks and switch between them.
    cartridge_ram: [u8; CARTRIDGE_RAM.len()],
    // ===== Registers =====
    /// `BANK`: Boot ROM mapping control
    ///
    /// If this is 0, the boot ROM is mapped over bytes `0x0-0x100`. If it's
    /// any other value, that range is mapped to the ROM instead.
    bank: u8,
}

impl RandomAccessMemory {
    fn ram(&self) -> MemoryView<'_> {
        MemoryView::from_slice(&self.ram, RAM)
    }

    fn high_ram(&self) -> MemoryView<'_> {
        MemoryView::from_slice(&self.high_ram, HIGH_RAM)
    }

    fn cartridge_ram(&self) -> MemoryView<'_> {
        MemoryView::from_slice(&self.cartridge_ram, CARTRIDGE_RAM)
    }

    /// Get the value of the `BANK` register
    #[cfg(test)]
    pub fn bank(&self) -> u8 {
        self.bank
    }
}

impl Default for RandomAccessMemory {
    fn default() -> Self {
        Self {
            bank: 0,
            ram: [0; RAM.len()],
            high_ram: [0; HIGH_RAM.len()],
            cartridge_ram: [0; CARTRIDGE_RAM.len()],
        }
    }
}
