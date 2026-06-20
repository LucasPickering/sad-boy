//! Emulation logic for a Nintendo Game Boy
//!
//! https://rylev.github.io/DMG-01/public/book/introduction.html

#![expect(unused)] // TODO remove this

use crate::rom::Rom;
use std::{io, path::Path};

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

    /// Update the emulator state based on an instruction
    fn execute(&mut self, instruction: Instruction) {
        match instruction {
            Instruction::Add(source) => self.add(self.get_value(source)),
        }
    }

    /// TODO
    fn get_value(&self, source: ValueSource) -> u8 {
        match source {
            ValueSource::A => self.registers.a,
            ValueSource::B => self.registers.b,
            ValueSource::C => self.registers.c,
            ValueSource::D => self.registers.d,
            ValueSource::E => self.registers.e,
            ValueSource::F => self.registers.f,
            ValueSource::H => self.registers.h,
            ValueSource::L => self.registers.l,
        }
    }

    /// Add a value to register `a`, setting flags as needed
    fn add(&mut self, value: u8) {
        let (sum, overflow) = self.registers.a.overflowing_add(value);
        let original = self.registers.a;
        self.registers.a = sum;
        self.registers.set_flags(Flags {
            zero: sum == 0,
            subtract: false,
            // Check if the bottom 4 bits overflowed into the top 4
            // TODO is this correct? write some prop tests
            half_carry: (sum & 0b1111) < (original & 0b1111),
            carry: overflow,
        });
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
    fn flags(&self) -> Flags {
        Flags::from_bits(self.f)
    }

    /// Set the `f` register to the given flags
    fn set_flags(&mut self, flags: Flags) {
        self.f = flags.into_bits();
    }
}

/// The `f` register can be interpreted as a set of 4 flags providing feedback
/// about the previous operation
///
/// Use [Registers::flags] to get this value.
///
/// https://rylev.github.io/DMG-01/public/book/cpu/registers.html#flags-register
#[derive(Copy, Clone)]
#[expect(clippy::struct_excessive_bools)]
struct Flags {
    /// TODO
    zero: bool,
    /// TODO
    subtract: bool,
    /// TODO
    half_carry: bool,
    /// TODO
    carry: bool,
}

impl Flags {
    /// Last operation resulted in a `0`
    const ZERO: u8 = 0b1 << 7;
    /// Last operation was a subtraction
    const SUBTRACT: u8 = 0b1 << 6;
    /// The bottom 4 bits overflowed into the top 4 in the last operation
    const HALF_CARRY: u8 = 0b1 << 5;
    /// Last operation overflowed (wrapped)
    const CARRY: u8 = 0b1 << 4;

    /// Read individual flags from the top 4 bits of the byte
    fn from_bits(bits: u8) -> Self {
        let flag = |bit: u8| bits & bit != 0;
        Flags {
            zero: flag(Flags::ZERO),
            subtract: flag(Flags::SUBTRACT),
            half_carry: flag(Flags::HALF_CARRY),
            carry: flag(Flags::CARRY),
        }
    }

    /// Convert individual flags into bitflags
    fn into_bits(self) -> u8 {
        let bit = |flag: bool, bit: u8| if flag { bit } else { 0 };
        bit(self.zero, Self::ZERO)
            | bit(self.subtract, Self::SUBTRACT)
            | bit(self.half_carry, Self::HALF_CARRY)
            | bit(self.carry, Self::CARRY)
    }
}

/// CPU instruction
enum Instruction {
    /// Add a constant/register value to register `a`
    Add(ValueSource),
}

/// TODO
enum ValueSource {
    A,
    B,
    C,
    D,
    E,
    F,
    H,
    L,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    fn zero(register: Flags) -> bool {
        register.zero
    }

    fn subtract(register: Flags) -> bool {
        register.subtract
    }

    fn half_carry(register: Flags) -> bool {
        register.half_carry
    }

    fn carry(register: Flags) -> bool {
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
        #[case] getter: impl FnOnce(Flags) -> bool,
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
