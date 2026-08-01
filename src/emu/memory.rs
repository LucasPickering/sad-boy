use crate::{
    emu::{
        gpu::Gpu,
        instruction::Instruction,
        rom::{self, Rom},
    },
    util::Shared,
};
use std::{
    any,
    cell::RefCell,
    collections::HashMap,
    fmt::{self, Debug, Display},
    mem, ptr,
    range::RangeInclusive,
};
use tracing::{error, info};

/// Code executed at boot, before entering the ROM
///
/// The last instruction executed by the bootloader will unmap itself by writing
/// to the `BANK` register (`0xFF50`).
///
/// https://gbdev.io/pandocs/Power_Up_Sequence.html
///
/// Downloaded from https://gbdev.gg8.se/files/roms/bootroms/
const BOOTLOADER_CODE: &[u8] = include_bytes!("../../bootloader/dmg_boot.bin");
// ===== Memory Blocks =====
// https://gbdev.io/pandocs/Memory_Map.html
/// Range containing the bootloader code
///
/// This range is mapped *on top of* the ROM code while the bootloader is
/// running.
pub const BOOTLOADER: AddressRange =
    AddressRange::new("Bootloader", 0x0000, 0x0100);
/// Range of CPU instructions and data from a game cartridge
pub const GAME_ROM: AddressRange = AddressRange::new("ROM", 0x0000, 0x7FFF);
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
    BOOTLOADER,
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
/// Some parts of accessible memory are held as references. This aliases each
/// component based on given memory addresses. This allows each component of
/// memory/registers/etc. to be owned by its relevant module and handed out to
/// the CPU only as needed.
///
/// https://gbdev.io/pandocs/Memory_Map.html
#[derive(Debug)]
pub struct MemoryBus<'a> {
    /// TODO
    registers: &'a mut Registers,
    /// General-purpose writable memory
    ///
    /// This is boxed because 8KiB is too big to reasonably put on the stack.
    ram: Memory<u8>,
    /// Additional general-purpose writable memory
    ///
    /// This is most commonly used when accessed by the `LD HL, SP+imm8`
    /// instruction.
    high_ram: Memory<u8>,
    /// Additional RAM provided by the cartridge
    ///
    /// This is similar to the other two RAM blocks, except some cartridges can
    /// provide multiple banks and switch between them.
    cartridge_ram: Memory<u8>,
    /// Read-only memory from the cartridge
    rom: &'a Rom,
    /// Graphics processing
    ///
    /// This holds VRAM and graphics-related IO registers. This reference is
    /// shared with a separate GPU task, so it's an immutable reference with
    /// internal mutability.
    gpu: &'a Gpu,
    /// An extremely naive cache for instructions parsed from the ROM
    instruction_cache: HashMap<Address, (Instruction, usize)>,
}

impl<'a> MemoryBus<'a> {
    pub fn new(
        rom: &'a Rom,
        registers: &'a mut Registers,
        gpu: &'a Gpu,
    ) -> Self {
        Self {
            registers,
            ram: Memory::new(RAM),
            high_ram: Memory::new(HIGH_RAM),
            cartridge_ram: Memory::new(CARTRIDGE_RAM),
            rom,
            gpu,
            instruction_cache: HashMap::new(),
        }
    }

    /// Load the CPU instruction at the given address
    ///
    /// Return the instruction as well as the number of bytes it consumed. This
    /// is the number of bytes that the PC should advance.
    pub fn get_instruction(
        &mut self,
        address: Address,
    ) -> (Instruction, usize) {
        // Instruction parsing is set up to read from either the bootloader or
        // the ROM. Reading from anywhere else is a bug.
        //
        // Since parsing requires a slice instead of accessing bytes one at a
        // time, this is easier than supporting instruction parsing from
        // arbitrary addresses.
        assert!(
            GAME_ROM.contains(address),
            "Requested instruction at {address} is out of range {GAME_ROM}"
        );

        // Cache instructions because parsing is expensive
        let is_bootloading = self.is_bootloading();
        *self.instruction_cache.entry(address).or_insert_with(|| {
            let source = if is_bootloading {
                // If the bootloader is enabled, then we should only be running
                // bootloader code. Out-of-bounds here implies the bootloader
                // exited without unmapping itself, or something
                // else re-mapped the bootloader.
                BOOTLOADER_CODE
            } else {
                self.rom.bytes()
            };
            rom::get_instruction(source, address).unwrap_or_else(|error| {
                panic!("Failed to parse instruction: {error}");
            })
        })
    }

    /// Get a 1-byte value from memory
    ///
    /// All 16-bit addresses are valid, so this is infallible.
    pub fn get8(&self, address: Address) -> u8 {
        /// Helper to get a GPU register
        macro_rules! gpu_reg {
            ($register:ident) => {
                self.gpu.registers().with(|r| r.$register.into())
            };
        }

        // https://gbdev.io/pandocs/Memory_Map.html
        match address.0 {
            BOOTLOADER_START..=BOOTLOADER_LAST if self.is_bootloading() => {
                let index: usize = address.0.into();
                BOOTLOADER_CODE[index]
            }
            // Game ROM
            // TODO consts for these
            0x0000..=0x3FFF => {
                // SAFETY: TODO
                let index: usize = address.0.into();
                self.rom.bytes()[index]
            }
            0x4000..=0x7FFF => {
                error!("TODO: Game ROM bank N");
                0
            }
            TILE_DATA_START..=TILE_DATA_LAST => {
                self.gpu.tile_data().byte(address)
            }
            TILE_MAPS_START..=TILE_MAPS_LAST => {
                self.gpu.tile_maps().byte(address)
            }
            CARTRIDGE_RAM_START..=CARTRIDGE_RAM_LAST => {
                self.cartridge_ram.byte(address)
            }
            RAM_START..=RAM_LAST => self.ram.byte(address),
            ECHO_RAM_START..=ECHO_RAM_LAST => {
                // TODO dedupe with set8
                // Make sure mirrored references can't go out of bounds
                debug_assert!(ECHO_RAM.len() <= RAM.len());
                // Shift to the main RAM section
                let address = Address(address.0 - ECHO_RAM_START + RAM_START);
                self.get8(address)
            }
            OAM_START..=OAM_LAST => self.gpu.oam().byte(address),
            0xFEA0..=0xFEFF => 0, // Null mem

            // Hardware registers
            LCDC => gpu_reg!(lcdc),
            STAT => gpu_reg!(stat),
            SCY => gpu_reg!(scy),
            SCX => gpu_reg!(scx),
            LY => gpu_reg!(ly),
            LYC => gpu_reg!(lyc),
            DMA => gpu_reg!(dma),
            BANK => self.registers.bank,
            0xFF00..=0xFF7F => {
                error!("TODO: unmapped I/O register {address}");
                0
            }

            HIGH_RAM_START..=HIGH_RAM_LAST => self.high_ram.byte(address),
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
        /// Helper to set a GPU register
        macro_rules! gpu_reg {
            ($register:ident) => {
                self.gpu
                    .registers()
                    .with_mut(|r| r.$register = value.into())
            };
        }

        // https://gbdev.io/pandocs/Memory_Map.html
        match address.0 {
            // ROM is immutable (bootloader too, so it doesn't matter if it's
            // mapped or not)
            0x0000..=0x7FFF => {} // TODO const for this
            TILE_DATA_START..=TILE_DATA_LAST => {
                self.gpu.tile_data().set_byte(address, value);
            }
            TILE_MAPS_START..=TILE_MAPS_LAST => {
                self.gpu.tile_maps().set_byte(address, value);
            }
            CARTRIDGE_RAM_START..=CARTRIDGE_RAM_LAST => {
                self.cartridge_ram.set_byte(address, value);
            }
            RAM_START..=RAM_LAST => self.ram.set_byte(address, value),
            ECHO_RAM_START..=ECHO_RAM_LAST => {
                // Make sure mirrored references can't go out of bounds
                debug_assert!(ECHO_RAM.len() <= RAM.len());
                // Shift to the main RAM section
                let address = Address(address.0 - ECHO_RAM_START + RAM_START);
                self.set8(address, value);
            }
            OAM_START..=OAM_LAST => self.gpu.oam().set_byte(address, value),
            0xFEA0..=0xFEFF => {} // Null mem

            // Hardware registers
            LCDC => gpu_reg!(lcdc),
            STAT => gpu_reg!(stat),
            SCY => gpu_reg!(scy),
            SCX => gpu_reg!(scx),
            LY => gpu_reg!(ly),
            LYC => gpu_reg!(lyc),
            DMA => gpu_reg!(dma),
            BANK => self.set_bank(value),
            0xFF00..=0xFF7F => error!("TODO: unmapped I/O register {address}"),

            HIGH_RAM_START..=HIGH_RAM_LAST => {
                self.high_ram.set_byte(address, value);
            }

            0xFFFF => error!("TODO: Interrupt Enabled Register write"),
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

    /// Set the value of the `BANK` register
    fn set_bank(&mut self, value: u8) {
        // If exiting the bootloader, clear the instruction cache because new
        // executable memory is loaded in that range
        if self.is_bootloading() && value > 0 {
            info!("Exiting bootloader");
            self.instruction_cache = HashMap::new();
        }
        self.registers.bank = value;
    }

    /// Is the bootloader currently mapped?
    ///
    /// This is `true` only during initial boot. The bootloader unmaps itself
    /// with its last instruction, at which point it should never be re-mapped.
    fn is_bootloading(&self) -> bool {
        self.registers.bank == 0
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
/// This can hold any type `T`, and can be treated as either a slice of `T`
/// **or** a slice of bytes. `T` must have a stable byte representation! Don't
/// forget `#[repr(C)]`.
///
/// Use this for data that is accessible via the memory bus. If memory doesn't
/// have any semantic meaning (e.g. general-purpose RAM), just use `Memory<u8>`.
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

    /// Get the inner slice of `T` values
    pub fn as_values(&self) -> &[T] {
        &self.memory
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
        let offset = (address.0 - self.range.start().0) as usize;
        // Double extra sanity check
        let len_bytes = mem::size_of_val(&*self.memory);
        debug_assert!(
            offset < len_bytes,
            "Offset {offset} >= byte length {len_bytes}"
        );
        offset
    }
}

impl<T> MemoryRead for &Memory<T> {
    fn byte(self, address: Address) -> u8 {
        let offset = self.byte_offset(address);
        let ptr = ptr::from_ref(&*self.memory).cast::<u8>();
        // SAFETY:
        // - byte_offset() ensures the offset is in range for self.memory
        // - u8 is the smallest type so we don't have to worry about alignment
        // - TODO how guarantee no padding?
        unsafe { *ptr.add(offset) }
    }
}

impl<T> MemoryWrite for &mut Memory<T> {
    fn set_byte(self, address: Address, value: u8) {
        let offset = self.byte_offset(address);
        let ptr = ptr::from_mut(&mut *self.memory).cast::<u8>();
        // SAFETY:
        // - byte_offset() ensures the offset is in range for self.memory
        // - u8 is the smallest type so we don't have to worry about alignment
        //   or corrupted bytes
        unsafe { *ptr.add(offset) = value }
    }
}

/// A container that provides read access to some chunk of addressable memory
///
/// This is separate from [MemoryWrite] to allow that trait to selectively
/// provide internal mutation.
pub trait MemoryRead {
    /// Get the byte at the given memory address
    fn byte(self, address: Address) -> u8;
}

/// A container that provides write access to some chunk of addressable memory
///
/// This takes `self` and is separate from [MemoryRead] so the implementor can
/// selectively provide internal mutability.
pub trait MemoryWrite {
    /// Set the value of the byte at the given memory address
    fn set_byte(self, address: Address, value: u8);
}

impl<T> MemoryRead for &RefCell<Memory<T>> {
    fn byte(self, address: Address) -> u8 {
        self.borrow().byte(address)
    }
}

impl<T> MemoryWrite for &RefCell<Memory<T>> {
    fn set_byte(self, address: Address, value: u8) {
        self.borrow_mut().set_byte(address, value);
    }
}

/// TODO move this
#[derive(Debug, Default)]
pub struct Registers {
    /// `BANK`: Boot ROM mapping control
    ///
    /// If this is 0, the boot ROM is mapped over bytes `0x0-0x100`. If it's
    /// any other value, that range is mapped to the ROM instead.
    pub bank: u8,
}
