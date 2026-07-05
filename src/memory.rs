use crate::{
    emu::{Address, Instruction},
    rom::{Rom, RomParseError},
};
use derive_more::{Display, Error};
use std::ops::Range;

/// Virtual memory map pointing to the various addressable components
///
/// https://rylev.github.io/DMG-01/public/book/memory_map.html
#[derive(Debug)]
pub struct MemoryMap {
    rom: Rom,
}

impl MemoryMap {
    /// Initialize the memory map
    pub fn new(rom: Rom) -> Self {
        Self { rom }
    }

    /// Load the CPU instruction at the given address
    ///
    /// Return the instruction as well as the number of bytes it consumed. This
    /// is the number of bytes that the PC should advance.
    pub fn get_instruction(
        &self,
        address: Address,
    ) -> Result<(Instruction, usize), MemoryError> {
        const VALID_RANGE: Range<Address> = Address(0)..Address(0x8000);
        if VALID_RANGE.contains(&address) {
            self.rom
                .get_instruction(address)
                .map_err(MemoryError::InstructionParse)
        } else {
            Err(MemoryError::OutOfBounds {
                address,
                bounds: VALID_RANGE,
            })
        }
    }

    /// Get a 2-byte value from memory
    pub fn get16(&self, address: Address) -> Result<u16, MemoryError> {
        todo!()
    }
}

/// Error while accessing memory
#[derive(Debug, Display, Error)]
pub enum MemoryError {
    /// TODO
    #[display("TODO")]
    InstructionParse(#[error(source)] RomParseError),
    /// Requested access to memory that either doesn't exist or doesn't serve
    /// the requested purpose
    ///
    /// For example, if you pass an address to [MemoryMap::get_instruction]
    /// that's outside the CPU instruction memory range, you'll get this error.
    #[display("Out of bounds: address {address} not in range [{}, {})", bounds.start, bounds.end)]
    OutOfBounds {
        /// Address that was requested
        address: Address,
        /// Range of valid addresses for the purpose
        bounds: Range<Address>,
    },
}
