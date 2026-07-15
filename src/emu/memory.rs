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
    ops::{Deref, DerefMut, RangeInclusive},
};
use tracing::error;

/// Range of CPU instructions and data from a game cartridge
const GAME_ROM: AddressRange = AddressRange::new("ROM", 0x0000, 0x7FFF);
/// Address range for general-purpose writable RAM
pub const RAM: AddressRange = AddressRange::new("RAM", 0xC000, 0xDFFF);
/// A mirror of RAM that *should* not be used by games
const ECHO_RAM: AddressRange = AddressRange::new("Echo RAM", 0xE000, 0xFDFF);
/// Address range for additional general-purpose writable RAM
const HIGH_RAM: AddressRange = AddressRange::new("High RAM", 0xFF80, 0xFFFE);
/// Video RAM containing tile pixel data
const TILE_DATA: AddressRange = AddressRange::new("Tile Data", 0x8000, 0x97FF);
/// Object Attribute Memory (part of VRAM)
const OAM: AddressRange = AddressRange::new("OAM", 0xFE00, 0xFE9F);

// Extra consts for where expressions aren't allowed
const RAM_START: u16 = RAM.start();
const RAM_END: u16 = RAM.end();
pub const RAM_LEN: usize = RAM.len();
const ECHO_RAM_START: u16 = ECHO_RAM.start();
const ECHO_RAM_END: u16 = ECHO_RAM.end();
const HIGH_RAM_START: u16 = HIGH_RAM.start();
const HIGH_RAM_END: u16 = HIGH_RAM.end();
pub const HIGH_RAM_LEN: usize = HIGH_RAM.len();
const TILE_DATA_START: u16 = TILE_DATA.start();
const TILE_DATA_END: u16 = TILE_DATA.end();
pub const TILE_DATA_LEN: usize = TILE_DATA.len();
const OAM_START: u16 = OAM.start();
const OAM_END: u16 = OAM.end();

/// An abstraction over the addessable range of memory
///
/// This holds references to all the parts of memory that can be access, and
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
    pub ram: &'a mut Memory<RAM_LEN>,
    /// Additional general-purpose writable memory
    ///
    /// This is most commonly used when accessed by the `LD HL, SP+imm8`
    /// instruction.
    pub high_ram: &'a mut Memory<HIGH_RAM_LEN>,
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
            TILE_DATA_START..=TILE_DATA_END => {
                // Safety: TODO
                let index: usize = address.0.into();
                &self.gpu.tile_data()[index]
            }
            0x9800..=0x9FFF => {
                error!("TODO: Tile map read");
                &0
            }
            0xA000..=0xBFFF => {
                error!("TODO: Cartridge RAM read");
                &0
            }
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
            OAM_START..=OAM_END => {
                error!("TODO: Object Attribute Memory read");
                &0
            }
            0xFEA0..=0xFEFF => &0,
            0xFF00..=0xFF7F => {
                error!("TODO: I/O register read");
                &0
            }
            HIGH_RAM_START..=HIGH_RAM_END => {
                // Safety: self.high_ram is initialized to the same length as
                // this range
                let index = (address.0 - HIGH_RAM.start()) as usize;
                &self.high_ram[index]
            }
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
            TILE_DATA_START..=TILE_DATA_END => {
                // Safety: self.tile_data is initialized to the same length
                // as this range
                let index: usize = address.0.into();
                Some(&mut self.gpu.tile_data_mut()[index])
            }
            0x9800..=0x9FFF => todo!("TODO: Tile map read"),
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
            OAM_START..=OAM_END => None, // Object Attribute Memory
            0xFEA0..=0xFEFF => None,
            0xFF00..=0xFF7F => {
                error!("unimplemented: I/O register write");
                None
            }
            HIGH_RAM_START..=HIGH_RAM_END => {
                // Safety: self.high_ram is initialized to the same length as
                // this range
                let index = (address.0 - HIGH_RAM.start()) as usize;
                Some(&mut self.high_ram[index])
            }
            0xFFFF => todo!("Interrupt Enabled Register"),
        }
    }
}

/// A range of memory addresses
#[derive(Debug)]
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

impl Display for AddressRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let range = &self.range;
        write!(f, "{} [{}, {}]", self.name, range.start(), range.end())
    }
}

/// A fixed-length block of memory
///
/// This is a newtype for a byte array. It provides better debug formatting.
pub struct Memory<const N: usize>(Box<[u8; N]>);

impl<const N: usize> Default for Memory<N> {
    fn default() -> Self {
        Self(Box::new([0; N]))
    }
}

impl<const N: usize> Debug for Memory<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Debug::fmt(&BytesDisplay::hex(&*self.0), f)
    }
}

impl<const N: usize> Deref for Memory<N> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

impl<const N: usize> DerefMut for Memory<N> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.0
    }
}
