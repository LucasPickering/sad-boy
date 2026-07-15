use crate::{
    emu::{
        gpu::Gpu,
        instruction::{Address, Instruction},
        rom::Rom,
    },
    util::BytesDisplay,
};
use std::{
    fmt::{self, Debug, Display},
    ops::{Index, IndexMut},
    range::RangeInclusive,
};
use tracing::error;

// ===== Memory Blocks =====
// https://gbdev.io/pandocs/Memory_Map.html
/// Range of CPU instructions and data from a game cartridge
const GAME_ROM: AddressRange = AddressRange::new("ROM", 0x0000, 0x7FFF);
/// Video RAM containing tile pixel data
pub const TILE_DATA: AddressRange =
    AddressRange::new("Tile Data", 0x8000, 0x97FF);
/// Video RAM containing both tile maps
pub const TILE_MAPS: AddressRange =
    AddressRange::new("Tile Maps", 0x9800, 0x9FFF);
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
pub const DMA: u16 = 0xFF46;

/// Generate `x_START` and `x_END` consts for a set of memory ranges
///
/// These consts are needed to use the start/end in pattern matching, where
/// complex expressions aren't allowed.
macro_rules! bounds {
    ($($range:expr),* $(,)?) => {
        paste::paste! {
            $(
                const [<$range _START>]: u16 = $range.start();
                const [<$range _LAST>]: u16 = $range.last();
            )*
        }
    };
}

// Generate extra consts for pattern matching
bounds!(TILE_DATA, TILE_MAPS, RAM, ECHO_RAM, OAM, HIGH_RAM);

/// An abstraction over the addessable range of memory
///
/// This holds references to all the parts of memory that can be accessed, and
/// aliases to each component based on given memory addresses. This allows each
/// component of memory/registers/etc. to be owned by its relevant module and
/// handed out to the CPU only as needed.
///
/// https://gbdev.io/pandocs/Memory_Map.html
#[derive(Debug)]
pub struct MemoryBus<'a> {
    /// Read-only memory from the cartridge
    pub rom: &'a Rom,
    /// General-purpose writable memory
    ///
    /// This is boxed because 8KiB is too big to reasonably put on the stack.
    pub ram: &'a mut Memory,
    /// Additional general-purpose writable memory
    ///
    /// This is most commonly used when accessed by the `LD HL, SP+imm8`
    /// instruction.
    pub high_ram: &'a mut Memory,
    /// GPU holds VRAM and graphics registers
    pub gpu: &'a mut Gpu,
}

impl MemoryBus<'_> {
    /// Load the CPU instruction at the given address
    ///
    /// Return the instruction as well as the number of bytes it consumed. This
    /// is the number of bytes that the PC should advance.
    pub fn get_instruction(&self, address: Address) -> (Instruction, usize) {
        if GAME_ROM.contains(address) {
            self.rom.get_instruction(address).unwrap_or_else(|error| {
                panic!("Failed to parse instruction: {error}");
            })
        } else {
            // Either the ROM is buggy (possible, but unlikely), or it's a bug
            // (more likely). Panic will make it more identifiable.
            panic!(
                "Requested instruction at {address} is out of range {GAME_ROM}"
            );
        }
    }

    /// Get a 1-byte value from memory
    ///
    /// All 16-bit addresses are valid, so this is infallible.
    pub fn get8(&self, address: Address) -> u8 {
        *self.get_ref(address)
    }

    /// Get a mutable reference to a 1-byte value in memory
    ///
    /// If the memory isn't writable, return `None`.
    pub fn get8_mut(&mut self, address: Address) -> Option<&mut u8> {
        self.get_ref_mut(address)
    }

    /// Set a 1-byte value in memory
    ///
    /// If the memory isn't writable, this does nothing.
    pub fn set8(&mut self, address: Address, value: u8) {
        if let Some(byte) = self.get_ref_mut(address) {
            *byte = value;
        } else {
            error!("Skipping write to read-only address {address}");
        }
    }

    /// Get a 2-byte value from memory
    pub fn get16(&self, address: Address) -> u16 {
        let low = self.get8(address);
        let high = self.get8(address.next());
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
        self.set8(address.next(), high);
    }

    /// Map an Game Boy [Address] into an address in real memory
    ///
    /// This will check which range the address is in, and find the
    /// corresponding byte in RAM/ROM/etc. accordingly.
    ///
    /// All 16-bit addresses are valid, so this is infallible.
    fn get_ref(&self, address: Address) -> &u8 {
        // https://rylev.github.io/DMG-01/public/book/memory_map.html
        match address.0 {
            0x0000..=0x3FFF => {
                // Safety: TODO
                let index: usize = address.0.into();
                &self.rom.bytes()[index]
            }
            0x4000..=0x7FFF => {
                error!("TODO: Game ROM bank N read");
                &0
            }
            TILE_DATA_START..=TILE_DATA_LAST => &self.gpu.tile_data()[address],
            TILE_MAPS_START..=TILE_MAPS_LAST => &self.gpu.tile_maps()[address],
            0xA000..=0xBFFF => {
                error!("TODO: Cartridge RAM read");
                &0
            }
            RAM_START..=RAM_LAST => &self.ram[address],
            ECHO_RAM_START..=ECHO_RAM_LAST => {
                // Make sure mirrored references can't go out of bounds
                debug_assert!(ECHO_RAM.len() <= RAM.len());
                // Shift to the main RAM section
                let address = Address(address.0 - ECHO_RAM_START + RAM_START);
                &self.ram[address]
            }
            OAM_START..=OAM_LAST => {
                error!("TODO: Object Attribute Memory read");
                &0
            }
            0xFEA0..=0xFEFF => &0, // Null mem

            // Hardware registers
            LCDC => &self.gpu.registers().lcdc,
            STAT => &self.gpu.registers().stat,
            SCY => &self.gpu.registers().scy,
            SCX => &self.gpu.registers().scx,
            DMA => &self.gpu.registers().dma,
            0xFF00..=0xFF7F => {
                error!("TODO: I/O register read");
                &0
            }

            HIGH_RAM_START..=HIGH_RAM_LAST => &self.high_ram[address],
            0xFFFF => {
                error!("TODO: Interrupt Enabled Register read");
                &0
            }
        }
    }

    /// Map an Game Boy [Address] to an a mutable reference to real memory
    ///
    /// Return `None` if the addressed memory is not writable.
    fn get_ref_mut(&mut self, address: Address) -> Option<&mut u8> {
        // TODO dedupe this with get_ref()
        match address.0 {
            0x0000..=0x7FFF => None, // Cartridge ROM
            TILE_DATA_START..=TILE_DATA_LAST => {
                Some(&mut self.gpu.tile_data_mut()[address])
            }
            TILE_MAPS_START..=TILE_MAPS_LAST => {
                Some(&mut self.gpu.tile_maps_mut()[address])
            }
            0xA000..=0xBFFF => todo!("Cartridge RAM"),
            RAM_START..=RAM_LAST => Some(&mut self.ram[address]),
            ECHO_RAM_START..=ECHO_RAM_LAST => {
                // Make sure mirrored references can't go out of bounds
                debug_assert!(ECHO_RAM.len() <= RAM.len());
                // Shift to the main RAM section
                // Safety: self.ram is LARGER than the echo RAM section
                // TODO move this into a helper fn on AddressRange
                let address = Address(address.0 - ECHO_RAM_START + RAM_START);
                Some(&mut self.ram[address])
            }
            OAM_START..=OAM_LAST => None, // Object Attribute Memory
            0xFEA0..=0xFEFF => None,      // Null mem

            // Hardware registers
            LCDC => Some(&mut self.gpu.registers_mut().lcdc),
            STAT => Some(&mut self.gpu.registers_mut().stat),
            SCY => Some(&mut self.gpu.registers_mut().scy),
            SCX => Some(&mut self.gpu.registers_mut().scx),
            DMA => Some(&mut self.gpu.registers_mut().dma),
            0xFF00..=0xFF7F => {
                error!("unimplemented: I/O register write");
                None
            }

            HIGH_RAM_START..=HIGH_RAM_LAST => Some(&mut self.high_ram[address]),
            0xFFFF => todo!("Interrupt Enabled Register"),
        }
    }
}

/// A range of memory addresses
#[derive(Clone, Copy, Debug)]
pub struct AddressRange {
    name: &'static str,
    range: RangeInclusive<Address>,
}

impl AddressRange {
    /// Empty address range
    #[cfg(test)]
    const ZERO: Self = Self::new("Zero", 0, 0);

    /// Define a range of memory
    pub const fn new(name: &'static str, start: u16, end: u16) -> Self {
        Self {
            name,
            range: RangeInclusive {
                start: Address(start),
                last: Address(end),
            },
        }
    }

    /// Get the number of bytes in the range
    const fn len(&self) -> usize {
        // The end is inclusive, so we need +1 to count it
        (self.range.last.0 - self.range.start.0 + 1) as usize
    }

    /// First address included in the range
    pub const fn start(&self) -> u16 {
        self.range.start.0
    }

    /// Last address included in the range
    pub const fn last(&self) -> u16 {
        self.range.last.0
    }

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

/// A fixed-length block of memory
///
/// TODO
pub struct Memory {
    /// Range of memory addresses covered by this block
    range: AddressRange,
    /// Fixed-length binary data
    ///
    /// The length could be known and fixed at compile time, but plumbing that
    /// around is tedious with Rust's limited const generics. This slice will
    /// only be allocated once, when the memory is initialized.
    ///
    /// Invariant: length is always equal to `self.range.len()`
    memory: Box<[u8]>,
}

impl Memory {
    /// Initialize a new fixed-length block of memory with all zeroes
    pub fn new(range: AddressRange) -> Self {
        Self {
            range,
            memory: vec![0; range.len()].into_boxed_slice(),
        }
    }

    /// Initialize a zero-length block of memory
    #[cfg(test)]
    pub fn zero() -> Self {
        Self::new(AddressRange::ZERO)
    }

    /// Get the index into `self.memory` for a memory address
    ///
    /// In debug, this panics if the address is out of range.
    fn index(&self, address: Address) -> usize {
        debug_assert!(
            self.range.contains(address),
            "Address {address} out of bounds {range}",
            range = self.range
        );
        (address.0 - self.range.start()) as usize
    }
}

impl Debug for Memory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Memory")
            .field("range", &self.range)
            .field("memory", &BytesDisplay::hex(&self.memory))
            .finish()
    }
}

impl Index<Address> for Memory {
    type Output = u8;

    fn index(&self, address: Address) -> &Self::Output {
        &self.memory[self.index(address)]
    }
}

impl IndexMut<Address> for Memory {
    fn index_mut(&mut self, address: Address) -> &mut Self::Output {
        &mut self.memory[self.index(address)]
    }
}
