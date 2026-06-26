//! Emulation logic for a Nintendo Game Boy
//!
//! https://rylev.github.io/DMG-01/public/book/introduction.html

#![expect(unused)] // TODO remove this

use crate::rom::Rom;
use color_eyre::eyre;
use log::debug;
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
    pub fn load_rom(&mut self, path: &Path) -> eyre::Result<()> {
        let _rom = Rom::load(path)?;
        Ok(())
    }

    /// Update the emulator state based on an instruction
    fn execute(&mut self, instruction: Instruction) {
        match instruction {
            Instruction::Nop => {}
            _ => todo!(),
        }
    }

    /// Resolve an 8-bit value
    fn get_value8(&self, value: Value8) -> u8 {
        match value {
            Value8::Register(Register8::A) => self.registers.a,
            Value8::Register(Register8::B) => self.registers.b,
            Value8::Register(Register8::C) => self.registers.c,
            Value8::Register(Register8::D) => self.registers.d,
            Value8::Register(Register8::E) => self.registers.e,
            Value8::Register(Register8::H) => self.registers.h,
            Value8::Register(Register8::L) => self.registers.l,
            Value8::Register(Register8::Hl) => {
                let pointer = self.registers.hl();
                todo!("resolve pointer")
            }
            Value8::Const(value) => value,
        }
    }

    /// Resolve a 16-bit value
    fn get_value16(&self, value: Register16) -> u16 {
        match value {
            Register16::Bc => self.registers.bc(),
            Register16::De => self.registers.de(),
            Register16::Hl => self.registers.hl(),
            Register16::Sp => self.registers.sp,
        }
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

    /// Stack pointer
    // TODO should be Address?
    sp: u16,
    /// Program counter
    // TODO should be Address?
    pc: u16,
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
///
/// https://gbdev.io/pandocs/CPU_Instruction_Set.html
#[derive(Copy, Clone, Debug)]
pub enum Instruction {
    /// Add a value to a register
    /// TODO flatten this?
    Add(Add),
    /// Add an offset to register `sp`
    AddSp(i8),
    /// Add a value plus the flag to register `a`
    AddCarry(Value8),
    /// Bitwise AND between `a` and another value (modifies `a`)
    And(Value8),
    /// Get a single bit from a register (output to the `zero` flag)
    Bit(Bit, Register8),
    /// Get a single bit from the byte pointed to by `hl` (output to the `zero
    /// flag`)
    BitHl(Bit),
    /// Push a new frame onto the stack, then set `pc` to that address
    Call {
        address: Address,
        /// If defined, only call if true
        condition: Option<ConditionCode>,
    },
    /// Complement (invert) carry flag
    Ccf,
    /// Compare register `a` with another value
    Cp(Value8),
    /// Complement (bitwise NOT) register `a`
    Cpl,
    /// Decimal Adjust Accumulator
    Daa,
    /// Decrement a value by 1
    Dec(Dec),
    /// Disable interrupts
    Di,
    /// Enable interrupts
    Ei,
    /// Enter CPU low-power consumption mode until an interrupt occurs
    Halt,
    /// Increment a value by 1
    Inc(Inc),
    /// Jump to another address in the code
    Jp(Jump),
    /// Jump a relative number of instructions in the code
    Jr {
        offset: i8,
        /// If defined, only jump when true
        condition: Option<ConditionCode>,
    },
    /// Move a value
    Ld(InstructionLd),
    /// TODO
    Math { operation: Math, target: MathTarget },
    /// No op
    Nop,
    /// Return from subroutine
    ///
    /// If the condition is defined, only return if it's true
    Ret(Option<ConditionCode>),
    /// Return from subroutine and enable interrupts
    Reti,
    /// Rotate register `a` left, through the carry flag
    Rla,
    /// Rotate register `a` left
    Rlca,
    /// Rotate register `a` right, through the carry flag
    Rra,
    /// Rotate register `a` right
    Rrca,
    /// Set carry flag
    Scf,
    /// Enter CPU low power mode
    Stop,
}

/// TODO
#[derive(Copy, Clone, Debug)]
pub enum Math {
    /// TODO
    Adc,
    /// TODO
    Add,
    /// TODO
    And,
    /// TODO
    Cp,
    /// TODO
    Or,
    /// TODO
    Sbc,
    /// TODO
    Sub,
    /// TODO
    Xor,
}

/// TODO
#[derive(Copy, Clone, Debug)]
pub enum MathTarget {
    /// Byte in a register
    Register(Register8),
    /// Byte pointed to by register `hl`
    Hl,
    /// Constant value
    Const(u8),
}

/// Variations of the `ADD` instruction
#[derive(Copy, Clone, Debug)]
pub enum Add {
    /// Add a 16-bit value to `hl`
    Hl(Register16),
    /// Add `sp` to `hl`
    HlSp,
}

/// Variations of the `DEC` (decrement) instruction
#[derive(Copy, Clone, Debug)]
pub enum Dec {
    /// Decrement an 8-bit register
    R8(Register8),
    /// Decrement a 16-bit register
    R16(Register16),
    /// Decrement the byte pointed to by `hl`
    Hl,
}

/// Variations of the `INC` (increment) instruction
#[derive(Copy, Clone, Debug)]
pub enum Inc {
    /// Increment an 8-bit register
    R8(Register8),
    /// Increment a 16-bit register
    R16(Register16),
    /// Increment the byte pointed to by `hl`
    Hl,
}

/// Variations of the `JP` (jump) instruction
#[derive(Copy, Clone, Debug)]
pub enum Jump {
    /// Jump to a specific memory address
    Address(Address),
    /// Jump to a specific memory address if the condition is true
    AddressCc(ConditionCode, Address),
    /// Jump to the address pointed to by `hl`
    Hl,
}

/// Variations of the `LD` (load) instruction
#[derive(Copy, Clone, Debug)]
pub enum InstructionLd {
    /// Load from `sp` to a memory address
    AddressSp { dest: Address },
}

/// Source of an 8-bit value
#[derive(Copy, Clone, Debug)]
pub enum Value8 {
    /// Value from a register
    Register(Register8),
    /// Constant value
    Const(u8),
}

impl From<u8> for Value8 {
    fn from(value: u8) -> Self {
        Self::Const(value)
    }
}

impl From<Register8> for Value8 {
    fn from(register: Register8) -> Self {
        Self::Register(register)
    }
}

/// 8-bit register value (excluding `f`)
///
/// `r8` on https://rgbds.gbdev.io/docs/v1.0.1/gbz80.7
#[derive(Copy, Clone, Debug)]
pub enum Register8 {
    A,
    B,
    C,
    D,
    E,
    H,
    /// Byte pointed to by the address in register `hl`
    Hl,
    L,
}

/// Name of an 16-bit register
///
/// `r16` on https://rgbds.gbdev.io/docs/v1.0.1/gbz80.7
#[derive(Copy, Clone, Debug)]
pub enum Register16 {
    /// Value in register `bc`
    Bc,
    /// Value in register `de`
    De,
    /// Value in register `hl`
    Hl,
    /// Value in register `sp`
    Sp,
}

/// Condition for a conditional jump or call
#[derive(Copy, Clone, Debug)]
pub enum ConditionCode {
    /// Execute if `zero` flag is set
    Z,
    /// Execute if `zero` flag is not set
    Nz,
    /// Execute if `carry` flag is set
    C,
    /// Execute if `carry` flag is not set
    Nc,
}

/// Index of a single bit in a byte
///
/// Value can be `0-7`
#[derive(Copy, Clone, Debug)]
pub struct Bit(u8);

/// Address of a byte of RAM
#[derive(Copy, Clone, Debug)]
pub struct Address(pub u16);

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
