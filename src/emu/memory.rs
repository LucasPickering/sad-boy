use crate::emu::{gpu::Gpu, instruction::Instruction, rom::Rom};
use std::{
    any,
    fmt::{self, Debug, Display},
    mem, ptr,
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
    pub ram: &'a mut Memory<u8>,
    /// Additional general-purpose writable memory
    ///
    /// This is most commonly used when accessed by the `LD HL, SP+imm8`
    /// instruction.
    pub high_ram: &'a mut Memory<u8>,
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
        let accessor = Self::accessor(address);
        (accessor.read)(self, address)
    }

    /// Get a mutable reference to a 1-byte value in memory
    ///
    /// If the memory isn't writable, return `None`.
    pub fn get8_mut(&mut self, address: Address) -> Option<&mut u8> {
        let accessor = Self::accessor(address);
        accessor.write.map(|f| f(self, address))
    }

    /// Set a 1-byte value in memory
    ///
    /// If the memory isn't writable, this does nothing.
    pub fn set8(&mut self, address: Address, value: u8) {
        if let Some(byte) = self.get8_mut(address) {
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

    /// Get an [Accessor] that maps a Game Boy [Address] to real memory
    ///
    /// All 16-bit addresses are valid, so this is infallible.
    fn accessor(address: Address) -> Accessor {
        // https://rylev.github.io/DMG-01/public/book/memory_map.html
        match address.0 {
            // Game ROM
            0x0000..=0x3FFF => Accessor::ro(|bus, address| {
                // Safety: TODO
                let index: usize = address.0.into();
                bus.rom.bytes()[index]
            }),
            0x4000..=0x7FFF => {
                error!("TODO: Game ROM bank N");
                Accessor::ro(|_, _| 0)
            }
            TILE_DATA_START..=TILE_DATA_LAST => Accessor::rw(
                |bus, address| bus.gpu.tile_data().byte(address),
                |bus, address| bus.gpu.tile_data_mut().byte_mut(address),
            ),
            TILE_MAPS_START..=TILE_MAPS_LAST => Accessor::rw(
                |bus, address| bus.gpu.tile_maps().byte(address),
                |bus, address| bus.gpu.tile_maps_mut().byte_mut(address),
            ),
            0xA000..=0xBFFF => {
                error!("TODO: Cartridge RAM read");
                Accessor::ro(|_, _| 0)
            }
            RAM_START..=RAM_LAST => Accessor::rw(
                |bus, address| bus.ram.byte(address),
                |bus, address| bus.ram.byte_mut(address),
            ),
            ECHO_RAM_START..=ECHO_RAM_LAST => {
                // Make sure mirrored references can't go out of bounds
                debug_assert!(ECHO_RAM.len() <= RAM.len());
                // Shift to the main RAM section
                let address = Address(address.0 - ECHO_RAM_START + RAM_START);
                Self::accessor(address)
            }
            OAM_START..=OAM_LAST => {
                error!("TODO: Object Attribute Memory read");
                Accessor::ro(|_, _| 0)
            }
            // Null mem
            0xFEA0..=0xFEFF => Accessor::ro(|_, _| 0),

            // Hardware registers
            LCDC => Accessor::rw(
                |bus, _| bus.gpu.registers().lcdc,
                |bus, _| &mut bus.gpu.registers_mut().lcdc,
            ),
            STAT => Accessor::rw(
                |bus, _| bus.gpu.registers().stat,
                |bus, _| &mut bus.gpu.registers_mut().stat,
            ),
            SCY => Accessor::rw(
                |bus, _| bus.gpu.registers().scy,
                |bus, _| &mut bus.gpu.registers_mut().scy,
            ),
            SCX => Accessor::rw(
                |bus, _| bus.gpu.registers().scx,
                |bus, _| &mut bus.gpu.registers_mut().scx,
            ),
            DMA => Accessor::rw(
                |bus, _| bus.gpu.registers().dma,
                |bus, _| &mut bus.gpu.registers_mut().dma,
            ),
            0xFF00..=0xFF7F => {
                error!("TODO: I/O register read");
                Accessor::ro(|_, _| 0)
            }

            HIGH_RAM_START..=HIGH_RAM_LAST => Accessor::rw(
                |bus, address| bus.high_ram.byte(address),
                |bus, address| bus.high_ram.byte_mut(address),
            ),
            0xFFFF => {
                error!("TODO: Interrupt Enabled Register read");
                Accessor::ro(|_, _| 0)
            }
        }
    }
}

/// Address of a byte of memory
///
/// The Game Boy memory range covers the entire `u16` range, so all addresses
/// are valid.
///
/// https://rylev.github.io/DMG-01/public/book/memory_map.html
#[derive(Clone, Copy, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct Address(pub u16);

impl Address {
    /// Get the next address after this one (+1 byte)
    ///
    /// Useful for accessing 16-bit values as two separate bytes.
    pub fn next(self) -> Self {
        // TODO check if self == 0xffff
        Self(self.0 + 1)
    }
}

impl Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(&self, f) // Defer to Display
    }
}

impl Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const ADDRESS_WIDTH: usize = 4;
        write!(f, "0x{:0>ADDRESS_WIDTH$X}", self.0)
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
#[derive(Debug)]
pub struct Memory<T> {
    /// Range of memory addresses covered by this block
    range: AddressRange,
    /// Fixed-length binary data
    ///
    /// The length could be known and fixed at compile time, but plumbing that
    /// around is tedious with Rust's limited const generics. This slice will
    /// only be allocated once, when the memory is initialized.
    ///
    /// Invariant: length is always equal to `self.range.len()`
    memory: Box<[T]>,
}

impl<T> Memory<T> {
    /// Initialize a new fixed-length block of memory with all zeroes
    pub fn new(range: AddressRange) -> Self
    where
        T: Clone + Default,
    {
        let len_bytes = range.len();
        let size = mem::size_of::<T>();
        debug_assert_eq!(
            len_bytes % size,
            0,
            "Memory length must be divisible by item size: \
            T={t}, len_bytes={len_bytes}, size={size}",
            t = any::type_name::<T>(),
        );
        let len_t = len_bytes / size;
        Self {
            range,
            memory: vec![T::default(); len_t].into_boxed_slice(),
        }
    }

    /// Initialize a zero-length block of memory
    #[cfg(test)]
    pub fn zero() -> Self {
        Self {
            range: AddressRange::ZERO,
            memory: Box::new([]),
        }
    }

    /// Get the byte at the given memory address
    pub fn byte(&self, address: Address) -> u8 {
        let offset = self.byte_offset(address);
        let ptr = ptr::from_ref(&*self.memory).cast::<u8>();
        // Safety:
        // - byte_offset() ensures the offset is in range for self.memory
        // - u8 is the smallest type so we don't have to worry about alignment
        //   or corrupted bytes
        unsafe { *ptr.add(offset) }
    }

    /// Get a mutable reference to the byte at the given memory address
    pub fn byte_mut(&mut self, address: Address) -> &mut u8 {
        let offset = self.byte_offset(address);
        let ptr = ptr::from_mut(&mut *self.memory).cast::<u8>();
        // Safety:
        // - byte_offset() ensures the offset is in range for self.memory
        // - u8 is the smallest type so we don't have to worry about alignment
        //   or corrupted bytes
        unsafe { &mut *ptr.add(offset) }
    }

    /// Translate a global memory address into an offset for a single byte in
    /// `self.memory`
    ///
    /// This panics if the address is out of range. The returned offset is
    /// guaranteed to be less than the **byte-length** of `self.memory`.
    fn byte_offset(&self, address: Address) -> usize {
        assert!(
            self.range.contains(address),
            "Address {address} out of bounds {range}",
            range = self.range
        );
        let offset = (address.0 - self.range.start()) as usize;
        // Double extra sanity check
        debug_assert!(offset < self.memory.len() * mem::size_of::<T>());
        offset
    }
}

type ReadAccessor = fn(&MemoryBus, Address) -> u8;
type WriteAccessor = for<'a> fn(&'a mut MemoryBus, Address) -> &'a mut u8;

/// A container for functions to extract a single byte value from the memory
/// bus
///
/// This maps a Game Boy [Address] to real memory. Its purpose is to deduplicate
/// const and mutable access, eliminating the need for two different match
/// statements over the entire memory range. Maybe a "better" way would be a
/// custom match macro, but I hate big macros because they break formatting.
///
/// Some memory is read-only, in which case there will be no mutable accessor.
struct Accessor {
    read: ReadAccessor,
    write: Option<WriteAccessor>,
}

impl Accessor {
    /// Read-only memory accessor
    fn ro(read: ReadAccessor) -> Self {
        Self { read, write: None }
    }

    /// Read-write memory accessor
    fn rw(read: ReadAccessor, write: WriteAccessor) -> Self {
        Self {
            read,
            write: Some(write),
        }
    }
}
