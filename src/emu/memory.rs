use crate::{
    emu::{
        gpu::Vram,
        instruction::Instruction,
        rom::{self, Rom},
    },
    util::IntDisplay,
};
use std::{
    fmt::{self, Debug, Display},
    mem,
    ops::{Add, AddAssign},
    ptr,
    range::RangeInclusive,
    slice,
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
    AddressRange::named("Bootstrap", 0x0000, 0x00FF);
/// Static portion of the cartridge ROM
///
/// This is the first bank of the ROM, which **cannot** be switched.
pub const CARTRIDGE_ROM_0: AddressRange =
    AddressRange::named("Cartridge ROM Bank 0", 0x0000, 0x3FFF);
/// Switchable bank of the cartridge ROM
///
/// A cartridge can have any number of secondary banks and switch between them.
pub const CARTRIDGE_ROM_N: AddressRange =
    AddressRange::named("Cartridge ROM Bank N", 0x4000, 0x7FFF);
/// All read-only cartridge memory
pub const CARTRIDGE_ROM: AddressRange =
    CARTRIDGE_ROM_0.union("Cartridge ROM", CARTRIDGE_ROM_N);
/// Video RAM containing tile pixel data
pub const TILE_DATA: AddressRange =
    AddressRange::named("Tile Data", 0x8000, 0x97FF);
/// Video RAM containing both tile maps
pub const TILE_MAPS: AddressRange =
    AddressRange::named("Tile Maps", 0x9800, 0x9FFF);
/// Switchable RAM bank provided by the cartridge
pub const CARTRIDGE_RAM: AddressRange =
    AddressRange::named("Cartridge RAM", 0xA000, 0xBFFF);
/// Address range for general-purpose writable RAM
pub const RAM: AddressRange = AddressRange::named("RAM", 0xC000, 0xDFFF);
/// A mirror of RAM that *should* not be used by games
pub const ECHO_RAM: AddressRange =
    AddressRange::named("Echo RAM", 0xE000, 0xFDFF);
/// Object Attribute Memory (part of VRAM)
pub const OAM: AddressRange = AddressRange::named("OAM", 0xFE00, 0xFE9F);
/// A set of single-byte registers stored in VRAM
pub const GPU_REGISTERS: AddressRange =
    AddressRange::named("GPU Registers", 0xFF40, 0xFF46);
/// Address range for additional general-purpose writable RAM
pub const HIGH_RAM: AddressRange =
    AddressRange::named("High RAM", 0xFF80, 0xFFFE);
// ===== Hardware Registers ====
// https://gbdev.io/pandocs/Hardware_Reg_List.html
const BANK: Address = Address(0xFF50);

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
    /// Read-only memory from the cartridge
    rom: &'a Rom,
    /// RAM and registers
    ram: &'a mut RandomAccessMemory,
    /// VRAM and graphics-related IO registers
    vram: &'a mut Vram,
}

impl<'a> MemoryBus<'a> {
    /// Construct a memory bus from references to each addressable component
    pub fn new(
        rom: &'a Rom,
        ram: &'a mut RandomAccessMemory,
        vram: &'a mut Vram,
    ) -> Self {
        Self { rom, ram, vram }
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
        self.read_only().get8(address)
    }

    /// Set a 1-byte value in memory
    ///
    /// If the memory isn't writable, this does nothing.
    pub fn set8(&mut self, address: Address, value: u8) {
        let block = self.read_only().get_block(address);
        block.set(self, address, value);
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
        self.read_only().is_bootstrapping()
    }

    /// Get a read-only memory view
    fn read_only(&self) -> MemoryBusReadOnly<'_> {
        MemoryBusReadOnly {
            ram: self.ram,
            rom: self.rom,
            vram: self.vram,
        }
    }
}

/// A read-only version of [MemoryBus] for the debugger
///
/// This has to be a separate type because the bus holds references, which can't
/// be generic over mutability.
#[derive(Debug)]
pub struct MemoryBusReadOnly<'a> {
    /// RAM and registers
    pub(super) ram: &'a RandomAccessMemory,
    /// Read-only memory from the cartridge
    pub(super) rom: &'a Rom,
    /// VRAM and graphics-related IO registers
    pub(super) vram: &'a Vram,
}

impl MemoryBusReadOnly<'_> {
    /// Get a 1-byte value from memory
    ///
    /// All 16-bit addresses are valid, so this is infallible.
    pub fn get8(&self, address: Address) -> u8 {
        let block = self.get_block(address);
        block.get(self, address)
    }

    /// TODO
    pub fn get_metadata(&self, address: Address) -> MemoryMetadata {
        let block = self.get_block(address);
        block.metadata(address)
    }

    /// Get the block of memory containing the given address
    fn get_block(&self, address: Address) -> &'static dyn MemoryBlock {
        *BLOCKS
            .iter()
            .find(|block| block.contains(self, address))
            .unwrap_or_else(|| panic!("Unmapped address: {address}"))
    }

    /// Is the bootstrapp currently mapped?
    ///
    /// This is `true` only during initial boot. The bootstrap unmaps itself
    /// with its last instruction, at which point it should never be re-mapped.
    pub fn is_bootstrapping(&self) -> bool {
        self.ram.bank == 0
    }
}

/// Information about a particular byte in memory
pub struct MemoryMetadata {
    /// Address of the described memory
    pub address: Address,
    /// Name of the block that this address points to
    pub block_name: &'static str,
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

/// Parse from a hex number (*without* the preceding `0x`)
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
    name: Option<&'static str>,
    range: RangeInclusive<Address>,
}

impl AddressRange {
    /// The full address range
    pub const ALL: Self = Self::named("All", 0, u16::MAX);

    /// Define a range of memory
    ///
    /// The given bounds are **inclusive**: `[start, last]`.
    pub const fn new(start: u16, last: u16) -> Self {
        Self {
            name: None,
            range: RangeInclusive {
                start: Address(start),
                last: Address(last),
            },
        }
    }

    /// Define a range of memory with a descriptive name for debugging
    ///
    /// The given bounds are **inclusive**: `[start, last]`.
    pub const fn named(name: &'static str, start: u16, last: u16) -> Self {
        Self {
            name: Some(name),
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
            name: Some(name),
            range: RangeInclusive {
                start: self.start(),
                last: other.last(),
            },
        }
    }

    /// Get the number of bytes in the range
    pub const fn len(&self) -> usize {
        // The end is inclusive, so we need +1 to count it
        (self.range.last.0 as usize) - (self.range.start.0 as usize) + 1
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
    /// addressed memory. **The returned offset is always less than
    /// `self.len()`.**
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
        if let Some(name) = self.name {
            write!(f, "{name} ")?;
        }
        write!(f, "[{}, {}]", range.start, range.last)
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

/// A marker trait denoting that a type can be cast to raw bytes
///
/// This serves two purposes:
/// - An added layer of safety to make sure types are opting in to providing a
///   stable memory layout
/// - Accessors can return `&impl RawBytes` to mask their return type, for cases
///   where the type is only needed for the memory bus
pub trait RawBytes {
    /// Access the value as immutable bytes
    fn as_bytes(&self) -> &[u8] {
        let byte_len = mem::size_of_val(self);
        // SAFETY:
        // - Pointer is valid because the corresponding slice is still alive
        // - Length is correct because it's calculated from the slice above
        // - range.offset() ensures the offset is in the address range, which is
        //   the same length as the slice
        let ptr = ptr::from_ref(self).cast::<u8>();
        unsafe { slice::from_raw_parts(ptr, byte_len) }
    }

    /// Access the value as mutable bytes
    fn as_bytes_mut(&mut self) -> &mut [u8] {
        let byte_len = mem::size_of_val(self);
        // SAFETY:
        // - Pointer is valid because the corresponding slice is still alive
        // - Length is correct because it's calculated from the slice above
        // - range.offset() ensures the offset is in the address range, which is
        //   the same length as the slice
        let ptr = ptr::from_mut(self).cast::<u8>();
        unsafe { slice::from_raw_parts_mut(ptr, byte_len) }
    }
}

impl RawBytes for [u8] {}

/// All available memory blocks
///
/// All memory lookups use this list to determine where each byte lives. It
/// covers the entire range: every possible address 0-65535 is part of a block.
///
/// https://gbdev.io/pandocs/Memory_Map.html
#[expect(clippy::redundant_closure_for_method_calls)]
const BLOCKS: &[&'static dyn MemoryBlock] = &[
    // SAFETY: BOOTSTRAP.len() == BOOTSTRAP_CODE.len()
    &ReadOnlyBytes::new(BOOTSTRAP, |_| BOOTSTRAP_CODE)
        // Unmap once the bootstrap is done
        .with_enabled(|bus| bus.is_bootstrapping()),
    // SAFETY: Cartridge ROM asserts its own length
    &ReadOnlyBytes::new(CARTRIDGE_ROM_0, |bus| bus.rom.bytes()),
    &PlaceholderBytes::new(CARTRIDGE_ROM_N),
    &OptionalBytes::new(
        TILE_DATA,
        |bus| bus.vram.tile_data().map(RawBytes::as_bytes),
        |bus| bus.vram.tile_data_mut().map(RawBytes::as_bytes_mut),
    ),
    &OptionalBytes::new(
        TILE_MAPS,
        |bus| bus.vram.tile_maps().map(RawBytes::as_bytes),
        |bus| bus.vram.tile_maps_mut().map(RawBytes::as_bytes_mut),
    ),
    &Bytes::new(
        CARTRIDGE_RAM,
        |bus| &bus.ram.cartridge_ram,
        |bus| &mut bus.ram.cartridge_ram,
    ),
    &Bytes::new(RAM, |bus| &bus.ram.ram, |bus| &mut bus.ram.ram),
    // Echo RAM is *smaller* than the original RAM being overlayed, so we can
    // just index into the normal RAM block
    &Bytes::new(ECHO_RAM, |bus| &bus.ram.ram, |bus| &mut bus.ram.ram),
    &OptionalBytes::new(
        OAM,
        |bus| bus.vram.oam().map(RawBytes::as_bytes),
        |bus| bus.vram.oam_mut().map(RawBytes::as_bytes_mut),
    ),
    &Null::new(AddressRange::named("Null", 0xFEA0, 0xFEFF)),
    &Bytes::new(
        GPU_REGISTERS,
        |bus| bus.vram.registers().as_bytes(),
        |bus| bus.vram.registers_mut().as_bytes_mut(),
    ),
    &PlaceholderBytes::new(AddressRange::named(
        "I/O registers",
        0xFF00,
        0xFF7F,
    )),
    &Byte::new("BANK", BANK, |bus| bus.ram.bank, |bus| &mut bus.ram.bank),
    &Bytes::new(
        HIGH_RAM,
        |bus| &bus.ram.high_ram,
        |bus| &mut bus.ram.high_ram,
    ),
    &PlaceholderBytes::new(AddressRange::named("INT enable", 0xFFFF, 0xFFFF)),
];

/// A trait for types that expose read/write functionality of a block of memory
///
/// There are a few different implementations depending on the layout/behavior
/// of the underlying memory.
trait MemoryBlock {
    /// Is the memory block accessible?
    ///
    /// Used only for the bootstrap ROM, which is disabled after completion.
    fn enabled(&self, _bus: &MemoryBusReadOnly) -> bool {
        true
    }

    /// Get the range of addresses covered by this block
    fn range(&self) -> AddressRange;

    /// Does this block contain the given address?
    ///
    /// This also checks [Self::enabled]; disabled blocks contain no addresses.
    fn contains(&self, bus: &MemoryBusReadOnly, address: Address) -> bool {
        self.enabled(bus) && self.range().contains(address)
    }

    /// Get a byte of memory at the given address
    ///
    /// This may panic if the address is outside [Self::range].
    fn get(&self, bus: &MemoryBusReadOnly, address: Address) -> u8;

    /// Set a byte of memory at the given address
    ///
    /// This may panic if the address is outside [Self::range].
    fn set(&self, bus: &mut MemoryBus, address: Address, value: u8);

    /// TODO
    fn metadata(&self, address: Address) -> MemoryMetadata;
}

/// [MemoryBlock] implementation for null bytes
///
/// Reads return 0, writes do nothing.
struct Null {
    range: AddressRange,
}

impl Null {
    const fn new(range: AddressRange) -> Self {
        Self { range }
    }
}

impl MemoryBlock for Null {
    fn range(&self) -> AddressRange {
        self.range
    }

    fn get(&self, _bus: &MemoryBusReadOnly, _address: Address) -> u8 {
        0
    }

    fn set(&self, _bus: &mut MemoryBus, _address: Address, _value: u8) {}

    fn metadata(&self, address: Address) -> MemoryMetadata {
        MemoryMetadata {
            address,
            block_name: self.range.name.unwrap_or("???"),
        }
    }
}

/// [MemoryBlock] implementation for single mutable byte (e.g. a register)
struct Byte {
    range: AddressRange,
    get: fn(&MemoryBusReadOnly) -> u8,
    get_mut: for<'a> fn(&'a mut MemoryBus) -> &'a mut u8,
}

impl Byte {
    const fn new(
        name: &'static str,
        address: Address,
        get: fn(&MemoryBusReadOnly) -> u8,
        get_mut: for<'a> fn(&'a mut MemoryBus) -> &'a mut u8,
    ) -> Self {
        Self {
            range: AddressRange::named(name, address.0, address.0),
            get,
            get_mut,
        }
    }
}

impl MemoryBlock for Byte {
    fn range(&self) -> AddressRange {
        self.range
    }

    fn get(&self, bus: &MemoryBusReadOnly, _address: Address) -> u8 {
        (self.get)(bus)
    }

    fn set(&self, bus: &mut MemoryBus, _address: Address, value: u8) {
        *(self.get_mut)(bus) = value;
    }

    fn metadata(&self, address: Address) -> MemoryMetadata {
        MemoryMetadata {
            address,
            block_name: self.range.name.unwrap_or("???"),
        }
    }
}

/// [MemoryBlock] implementation for a read-only byte slice
///
/// The caller defines how to extract the byte slice from the bus, and this
/// struct handles the rest.
struct ReadOnlyBytes {
    range: AddressRange,
    get: for<'a> fn(&'a MemoryBusReadOnly) -> &'a [u8],
    /// Used for the bootstrap ROM to unmap itself
    enabled: fn(&MemoryBusReadOnly) -> bool,
}

impl ReadOnlyBytes {
    const fn new(
        range: AddressRange,
        get: for<'a> fn(&'a MemoryBusReadOnly) -> &'a [u8],
    ) -> Self {
        Self {
            range,
            get,
            enabled: |_| true,
        }
    }

    /// Set the function used to enable/disable this block
    const fn with_enabled(
        mut self,
        enabled: fn(&MemoryBusReadOnly) -> bool,
    ) -> Self {
        self.enabled = enabled;
        self
    }
}

impl MemoryBlock for ReadOnlyBytes {
    fn range(&self) -> AddressRange {
        self.range
    }

    fn get(&self, bus: &MemoryBusReadOnly, address: Address) -> u8 {
        let bytes = (self.get)(bus);
        bytes[self.range.offset(address)]
    }

    fn set(&self, _bus: &mut MemoryBus, _address: Address, _value: u8) {}

    fn metadata(&self, address: Address) -> MemoryMetadata {
        MemoryMetadata {
            address,
            block_name: self.range.name.unwrap_or("???"),
        }
    }
}

/// [MemoryBlock] implementation for a mutable byte slice (e.g. RAM)
///
/// The caller defines how to extract the byte slice from the bus, and this
/// struct handles the rest.
struct Bytes {
    range: AddressRange,
    get: for<'a> fn(&'a MemoryBusReadOnly) -> &'a [u8],
    get_mut: for<'a> fn(&'a mut MemoryBus) -> &'a mut [u8],
}

impl Bytes {
    const fn new(
        range: AddressRange,
        get: for<'a> fn(&'a MemoryBusReadOnly) -> &'a [u8],
        get_mut: for<'a> fn(&'a mut MemoryBus) -> &'a mut [u8],
    ) -> Self {
        Self {
            range,
            get,
            get_mut,
        }
    }
}

impl MemoryBlock for Bytes {
    fn range(&self) -> AddressRange {
        self.range
    }

    fn get(&self, bus: &MemoryBusReadOnly, address: Address) -> u8 {
        let bytes = (self.get)(bus);
        bytes[self.range.offset(address)]
    }

    fn set(&self, bus: &mut MemoryBus, address: Address, value: u8) {
        let bytes = (self.get_mut)(bus);
        bytes[self.range.offset(address)] = value;
    }

    fn metadata(&self, address: Address) -> MemoryMetadata {
        MemoryMetadata {
            address,
            block_name: self.range.name.unwrap_or("???"),
        }
    }
}

/// [MemoryBlock] implementation for a mutable byte slice that may not be
/// present
///
/// The caller defines how to extract the byte slice from the bus, and this
/// struct handles the rest. If the memory block is `None`, gets will return `0`
/// and sets do nothing. The VRAM blocks use this to make themselves
/// inaccessible during certain modes.
struct OptionalBytes {
    range: AddressRange,
    get: for<'a> fn(&'a MemoryBusReadOnly) -> Option<&'a [u8]>,
    get_mut: for<'a> fn(&'a mut MemoryBus) -> Option<&'a mut [u8]>,
}

impl OptionalBytes {
    const fn new(
        range: AddressRange,
        get: for<'a> fn(&'a MemoryBusReadOnly) -> Option<&'a [u8]>,
        get_mut: for<'a> fn(&'a mut MemoryBus) -> Option<&'a mut [u8]>,
    ) -> Self {
        Self {
            range,
            get,
            get_mut,
        }
    }
}

impl MemoryBlock for OptionalBytes {
    fn range(&self) -> AddressRange {
        self.range
    }

    fn get(&self, bus: &MemoryBusReadOnly, address: Address) -> u8 {
        match (self.get)(bus) {
            Some(bytes) => bytes[self.range.offset(address)],
            None => 0,
        }
    }

    fn set(&self, bus: &mut MemoryBus, address: Address, value: u8) {
        if let Some(bytes) = (self.get_mut)(bus) {
            bytes[self.range.offset(address)] = value;
        }
    }

    fn metadata(&self, address: Address) -> MemoryMetadata {
        MemoryMetadata {
            address,
            block_name: self.range.name.unwrap_or("???"),
        }
    }
}

/// Placeholder for memory ranges I haven't implemented yet
struct PlaceholderBytes {
    range: AddressRange,
}

impl PlaceholderBytes {
    const fn new(range: AddressRange) -> Self {
        Self { range }
    }
}

impl MemoryBlock for PlaceholderBytes {
    fn range(&self) -> AddressRange {
        self.range
    }

    fn get(&self, _bus: &MemoryBusReadOnly, address: Address) -> u8 {
        error!(
            "TODO: unmapped read in {name}: {address}",
            name = self.range.name.unwrap_or("???")
        );
        0
    }

    fn set(&self, _bus: &mut MemoryBus, address: Address, _value: u8) {
        error!(
            "TODO: unmapped write in {name}: {address}",
            name = self.range.name.unwrap_or("???")
        );
    }

    fn metadata(&self, address: Address) -> MemoryMetadata {
        MemoryMetadata {
            address,
            block_name: self.range.name.unwrap_or("???"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ensure that every address in the valid memory range (`[0, 65535]`) is
    /// mapped to exactly one range
    #[test]
    fn range_coverage() {
        let rom = Rom::empty();
        let mut memory = RandomAccessMemory::default();
        let mut vram = Vram::default();
        let mut memory = MemoryBus::new(&rom, &mut memory, &mut vram);
        for address in 0..=u16::MAX {
            let address = Address(address);
            memory.get8(address);
            memory.set8(address, 0);
        }
    }
}
