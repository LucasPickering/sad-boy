//! Emulation logic for a Nintendo Game Boy
//!
//! https://rylev.github.io/DMG-01/public/book/introduction.html

use crate::rom::Rom;
use std::{io, marker::PhantomData, path::Path};

/// Game Boy emulator
#[derive(Debug, Default)]
pub struct GameBoy {
    registers: Registers,
}

impl GameBoy {
    /// Create a new emulator
    pub fn new() -> Self {
        Self::default()
    }

    /// Load a ROM from a file and begin running it
    pub fn load_rom(&mut self, path: &Path) -> io::Result<()> {
        let _rom = Rom::load(path)?;
        Ok(())
    }
}

/// Registers in a Game Boy CPU
#[derive(Debug, Default)]
#[repr(C)] // Field ordering/alignment is important
struct Registers {
    // Registers are ordered so pairs are kept together. This allows them to be
    // accessed as separate bytes or a pair together
    // af
    a: u8,
    f: u8,
    // bc
    b: u8,
    c: u8,
    // de
    d: u8,
    e: u8,
    // hl
    h: u8,
    l: u8,
}

/// Generate methods on [Registers] to access two registers as a 16-bit value
///
/// The methods use unsafe operations to treat the two registers as a single
/// value. For that reason, **field order on [Registers] is extremely
/// important.** The pointer to the first register of the pair is case from a
/// `u8` pointer to a `u16` pointer; the second register is **assumed** to
/// be the following byte in memory.
macro_rules! register_pair {
    ($pair:ident, $pair_mut:ident, $r1:ident) => {
        /// Get the value of the `$pair` register pair
        fn $pair(&self) -> u16 {
            // SAFETY: TODO
            #[expect(clippy::cast_ptr_alignment, clippy::ptr_as_ptr)]
            unsafe {
                *((&raw const self.$r1) as *const u16)
            }
        }

        /// Get a mutable reference to the `$pair` register pair
        fn $pair_mut(&mut self) -> &mut u16 {
            // SAFETY: TODO
            #[expect(clippy::cast_ptr_alignment, clippy::ptr_as_ptr)]
            unsafe {
                &mut *((&raw mut self.$r1) as *mut u16)
            }
        }
    };
}

impl Registers {
    register_pair!(af, af_mut, a);
    register_pair!(bc, bc_mut, b);
    register_pair!(de, de_mut, d);
    register_pair!(hl, hl_mut, h);

    /// Read bit flags from the `f` register
    fn flags(&self) -> FlagsRegister {
        let bit = |bit: u8| (self.f >> bit) & 0b1 != 0;
        FlagsRegister {
            zero: bit(7),
            subtract: bit(6),
            half_carry: bit(5),
            carry: bit(4),
        }
    }
}

/// The `f` register can be interpreted as a set of 4 flags providing feedback
/// about the previous operation
///
/// Use [Registers::flags] to get this value.
///
/// https://rylev.github.io/DMG-01/public/book/cpu/registers.html#flags-register
#[expect(clippy::struct_excessive_bools)]
struct FlagsRegister {
    /// TODO
    zero: bool,
    /// TODO
    subtract: bool,
    /// TODO
    half_carry: bool,
    /// TODO
    carry: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    fn zero(register: FlagsRegister) -> bool {
        register.zero
    }

    fn subtract(register: FlagsRegister) -> bool {
        register.subtract
    }

    fn half_carry(register: FlagsRegister) -> bool {
        register.half_carry
    }

    fn carry(register: FlagsRegister) -> bool {
        register.carry
    }

    /// Test [Registers::flags]
    #[rstest]
    #[case::zero_false(zero, 0b0111_0000, false)]
    #[case::zero_true(zero, 0b1000_0000, true)]
    #[case::subtract_false(subtract, 0b1011_0000, false)]
    #[case::subtract_true(subtract, 0b0100_0000, true)]
    #[case::half_carry_false(half_carry, 0b1101_0000, false)]
    #[case::half_carry_true(half_carry, 0b0010_0000, true)]
    #[case::carry_false(carry, 0b1110_0000, false)]
    #[case::carry_true(carry, 0b0001_0000, true)]
    fn flags(
        #[case] getter: impl FnOnce(FlagsRegister) -> bool,
        #[case] register_value: u8,
        #[case] expected: bool,
    ) {
        let registers = Registers {
            f: register_value,
            ..Default::default()
        };
        let actual = getter(registers.flags());
        assert_eq!(actual, expected);
    }
}
