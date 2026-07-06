use crate::{
    instruction::{Address, Instruction},
    rom::{Rom, RomParseError},
};
use derive_more::{Display, Error};
use std::ops::RangeInclusive;
use tracing::error;

/// Range of CPU instructions and data from a game cartridge
const GAME_ROM: AddressRange = AddressRange::new("ROM", 0x0000, 0x7FFF);
/// Address range for general-purpose writable RAM
pub const RAM: AddressRange = AddressRange::new("RAM", 0xC000, 0xDFFF);
/// A mirror of RAM that *should* not be used by games
const ECHO_RAM: AddressRange = AddressRange::new("Echo RAM", 0xE000, 0xFDFF);
/// Address range for additional general-purpose writable RAM
const HIGH_RAM: AddressRange = AddressRange::new("High RAM", 0xFF80, 0xFFFE);

// Extra consts for pattern matching
const RAM_START: u16 = RAM.start();
const RAM_END: u16 = RAM.end();
const ECHO_RAM_START: u16 = ECHO_RAM.start();
const ECHO_RAM_END: u16 = ECHO_RAM.end();
const HIGH_RAM_START: u16 = HIGH_RAM.start();
const HIGH_RAM_END: u16 = HIGH_RAM.end();

/// Virtual memory map pointing to the various addressable components
///
/// https://rylev.github.io/DMG-01/public/book/memory_map.html
#[derive(Debug)]
pub struct MemoryMap {
    /// Read-only memory from the cartridge
    rom: Rom,
    /// General-purpose writable memory
    ///
    /// This is boxed because 8KiB is too big to reasonably put on the stack.
    ram: Box<[u8; RAM.len()]>,
    /// Additional general-purpose writable memory
    ///
    /// This is most commonly used when accessed by the `LD HL, SP+imm8`
    /// instruction.
    high_ram: Box<[u8; HIGH_RAM.len()]>,
}

impl MemoryMap {
    /// Initialize the memory map
    pub fn new(rom: Rom) -> Self {
        Self {
            rom,
            ram: Box::new([0; RAM.len()]),
            high_ram: Box::new([0; HIGH_RAM.len()]),
        }
    }

    /// Load the CPU instruction at the given address
    ///
    /// Return the instruction as well as the number of bytes it consumed. This
    /// is the number of bytes that the PC should advance.
    pub fn get_instruction(
        &self,
        address: Address,
    ) -> Result<(Instruction, usize), MemoryError> {
        Self::check_bounds(address, GAME_ROM)?;
        self.rom
            .get_instruction(address)
            .map_err(MemoryError::InstructionParse)
    }

    /// Get a 1-byte value from memory
    ///
    /// All 16-bit addresses are valid, so this is infallible.
    pub fn get8(&self, address: Address) -> u8 {
        *self.get_ref(address)
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
            0x4000..=0x7FFF => todo!("Game ROM bank N"),
            0x8000..=0x97FF => todo!("Tile RAM"),
            0x9800..=0x9FFF => todo!("Background Map"),
            0xA000..=0xBFFF => todo!("Cartridge RAM"),
            RAM_START..=RAM_END => {
                // Safety: self.ram is initialized to the same length as
                // this range
                let index = (address.0 - RAM.start()) as usize;
                &self.ram[index]
            }
            ECHO_RAM_START..=ECHO_RAM_END => {
                // Make sure mirrored references can't go out of bounds
                debug_assert!(ECHO_RAM.len() <= RAM.len());
                let index = (address.0 - ECHO_RAM_START) as usize;
                &self.ram[index]
            }
            0xFE00..=0xFE9F => todo!("Object Attribute Memory"),
            0xFEA0..=0xFEFF => &0,
            0xFF00..=0xFF7F => todo!("I/O Registers"),
            HIGH_RAM_START..=HIGH_RAM_END => {
                // Safety: self.high_ram is initialized to the same length as
                // this range
                let index = (address.0 - HIGH_RAM.start()) as usize;
                &self.high_ram[index]
            }
            0xFFFF => todo!("Interrupt Enabled Register"),
        }
    }

    /// Map an Game Boy [Address] to an a mutable reference to real memory
    ///
    /// Return `None` if the addressed memory is not writable.
    fn get_ref_mut(&mut self, address: Address) -> Option<&mut u8> {
        // TODO dedupe this with get_ref()
        match address.0 {
            0x0000..=0x9FFF => None, // Cartridge ROM
            0xA000..=0xBFFF => todo!("Cartridge RAM"),
            RAM_START..=RAM_END => {
                // Safety: self.ram is initialized to the same length as
                // this range
                let index = (address.0 - RAM.start()) as usize;
                Some(&mut self.ram[index])
            }
            ECHO_RAM_START..=ECHO_RAM_END => {
                // Make sure mirrored references can't go out of bounds
                debug_assert!(ECHO_RAM.len() <= RAM.len());
                // Echo RAM
                // Safety: self.ram is LARGER than the echo RAM section
                let index = (address.0 - ECHO_RAM_START) as usize;
                Some(&mut self.ram[index])
            }
            0xFE00..=0xFE9F => None, // Object Attribute Memory
            0xFEA0..=0xFEFF => None,
            0xFF00..=0xFF7F => todo!("I/O Registers"),
            HIGH_RAM_START..=HIGH_RAM_END => {
                // Safety: self.high_ram is initialized to the same length as
                // this range
                let index = (address.0 - HIGH_RAM.start()) as usize;
                Some(&mut self.high_ram[index])
            }
            0xFFFF => todo!("Interrupt Enabled Register"),
        }
    }
    /// Check if the address is in the given range, returning
    /// [MemoryError::OutOfBounds] if not
    fn check_bounds(
        address: Address,
        range: AddressRange,
    ) -> Result<(), MemoryError> {
        if range.contains(address) {
            Ok(())
        } else {
            Err(MemoryError::OutOfBounds { address, range })
        }
    }
}

/// Error while accessing memory
#[derive(Debug, Display, Error)]
pub enum MemoryError {
    /// TODO
    #[display("{_0}")]
    InstructionParse(#[error(source)] RomParseError),
    /// Requested access to memory that either doesn't exist or doesn't serve
    /// the requested purpose
    ///
    /// For example, if you pass an address to [MemoryMap::get_instruction]
    /// that's outside the CPU instruction memory range, you'll get this error.
    #[display("Out of bounds: address {address} not in range {range}")]
    OutOfBounds {
        /// Address that was requested
        address: Address,
        /// Range of valid addresses for the purpose
        range: AddressRange,
    },
}

/// A range of memory addresses
#[derive(Debug, Display)]
#[display("{name} [{}, {}]", range.start(), range.end())]
pub struct AddressRange {
    name: &'static str,
    range: RangeInclusive<Address>,
}

impl AddressRange {
    /// Define a range of memory
    pub const fn new(name: &'static str, start: u16, end: u16) -> Self {
        Self {
            name,
            range: Address(start)..=Address(end),
        }
    }

    /// Get the number of bytes in the range
    const fn len(&self) -> usize {
        // The end is inclusive, so we need +1 to count it
        (self.range.end().0 - self.range.start().0 + 1) as usize
    }

    pub const fn start(&self) -> u16 {
        self.range.start().0
    }

    pub const fn end(&self) -> u16 {
        self.range.end().0
    }

    pub fn contains(&self, address: Address) -> bool {
        self.range.contains(&address)
    }
}
