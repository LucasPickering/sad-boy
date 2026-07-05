//! Emulation logic for a Nintendo Game Boy
//!
//! https://rylev.github.io/DMG-01/public/book/introduction.html

#![expect(unused)] // TODO remove this

use crate::{
    memory::{MemoryError, MemoryMap},
    rom::Rom,
};
use color_eyre::eyre;
use log::warn;
use std::{
    fmt::Display,
    io,
    path::Path,
    thread,
    time::{Duration, Instant},
};

/// Game Boy emulator
#[derive(derive_more::Debug)]
pub struct GameBoy {
    registers: Registers,
    /// Virtual memory map
    #[debug(skip)]
    memory: MemoryMap,
}

impl GameBoy {
    /// Boot the Game Boy and load the ROM from a file
    pub fn boot(path: &Path) -> eyre::Result<Self> {
        let rom = Rom::load(path)?;
        let memory = MemoryMap::new(rom);
        Ok(Self {
            registers: Registers::default(),
            memory,
        })
    }

    /// Keep running until the CPU is halted
    pub fn run(&mut self) -> eyre::Result<()> {
        /// TODO explain
        const CYCLES_PER_FRAME: usize = 70224;
        let frame_time = Duration::from_secs_f64(1.0 / 60.0);

        loop {
            // https://josaphat.co/posts/gameboy-emulator/
            let mut cycle_budget = CYCLES_PER_FRAME;
            let frame_start = Instant::now();

            while cycle_budget > 0 {
                let (instruction, num_bytes) =
                    self.memory.get_instruction(self.registers.pc)?;
                let pc = self.registers.pc;
                let cycles = self.execute(instruction)?;
                cycle_budget = cycle_budget.saturating_sub(cycles);
                // If the instruction didn't modify the PC (e.g. jumps), then
                // advance it automatically
                if self.registers.pc == pc {
                    self.registers.pc.0 += num_bytes as u16;
                }
            }

            // Sleep for the rest of the frame
            // It's possible this sleeps _too_ long, but the difference should
            // be negligible.
            // Unstable: use sleep_until
            // https://github.com/rust-lang/rust/issues/113752
            let elapsed = frame_start.elapsed();
            if let Some(sleep_time) = frame_time.checked_sub(elapsed) {
                thread::sleep(sleep_time);
            }
        }
    }

    /// Execute a single CPU instruction, returning the number of consumed CPU
    /// cycles
    fn execute(
        &mut self,
        instruction: Instruction,
    ) -> Result<usize, MemoryError> {
        match instruction {
            Instruction::Nop => Ok(1),
            Instruction::Dec(dec_inc) => Ok(self.dec_inc(dec_inc, -1)?),
            Instruction::Inc(dec_inc) => Ok(self.dec_inc(dec_inc, 1)?),
            Instruction::Jp(jump) => Ok(self.jump(jump)),
            Instruction::Ld(load) => self.load(load),
            _ => {
                warn!("Unknown instruction {instruction:?}");
                Ok(1)
            }
        }
    }

    /// Execute a `DEC` or `INC` instruction
    ///
    /// `delta` should be `-1` for `DEC`, `1` for `INC` Return the number of
    /// consumed CPU cycles.
    fn dec_inc(
        &mut self,
        dec_inc: DecInc,
        delta: i8,
    ) -> Result<usize, MemoryError> {
        // TODO set flags
        match dec_inc {
            DecInc::R8(register) => {
                let register = self.register8_mut(register);
                *register = register.wrapping_add_signed(delta);
                Ok(1)
            }
            DecInc::R16(register) => {
                let register = self.register16_mut(register);
                *register = register.wrapping_add_signed(delta.into());
                Ok(2)
            }
            DecInc::Hl => {
                let value = self.hl_mem_mut()?;
                *value = value.wrapping_add_signed(delta);
                Ok(3)
            }
        }
    }

    /// Execute a `JP` instruction
    ///
    /// Return the number of consumed CPU cycles.
    fn jump(&mut self, jump: Jump) -> usize {
        match jump {
            Jump::Address(address) => {
                self.registers.pc = address;
                4
            }
            Jump::AddressCc(condition, address) => {
                if self.condition(condition) {
                    self.registers.pc = address;
                    4
                } else {
                    3
                }
            }
            Jump::Hl => {
                self.registers.pc = Address(self.registers.hl());
                1
            }
        }
    }

    /// Execute an `LD` instruction
    ///
    /// Return the number of consumed CPU cycles.
    fn load(&mut self, load: Load) -> Result<usize, MemoryError> {
        match load {
            Load::AddressA { dest } => {
                *self.memory.get8_mut(dest)? = self.registers.a;
                Ok(4)
            }
            Load::AAddress { source } => {
                self.registers.a = self.memory.get8(source)?;
                Ok(4)
            }
            Load::AddressSp { dest } => {
                *self.memory.get16_mut(dest)? = self.registers.sp.0;
                Ok(5)
            }
            Load::HlSpOffset { offset } => {
                let value =
                    self.registers.sp.0.wrapping_add_signed(offset.into());
                *self.registers.hl_mut() = value;
                // TODO set flags here
                // https://rgbds.gbdev.io/docs/v1.0.1/gbz80.7#LD_HL,SP+e8
                Ok(3)
            }
            Load::SpHl => {
                self.registers.sp = Address(self.registers.hl());
                Ok(2)
            }
            Load::R8Const { dest, source } => {
                *self.register8_mut(dest) = source;
                Ok(2)
            }
            Load::R8R8 { dest, source } => {
                *self.register8_mut(dest) = self.register8(source);
                Ok(1)
            }
            Load::R16Const { dest, source } => {
                *self.register16_mut(dest) = source;
                Ok(3)
            }
            Load::R16MemA { dest } => {
                let dest = Address(self.register16_mem(dest));
                *self.memory.get8_mut(dest)? = self.registers.a;
                Ok(2)
            }
            Load::AR16Mem { source } => {
                let source = Address(self.register16_mem(source));
                self.registers.a = self.memory.get8(source)?;
                Ok(2)
            }
            _ => {
                warn!("Unknown load: {load:?}");
                Ok(1)
            }
        }
    }

    /// Evaluate a [ConditionCode]
    fn condition(&self, condition: ConditionCode) -> bool {
        let flags = self.registers.flags();
        match condition {
            ConditionCode::Z => flags.zero,
            ConditionCode::Nz => !flags.zero,
            ConditionCode::C => flags.carry,
            ConditionCode::Nc => !flags.carry,
        }
    }

    /// Get a the value of an 8-bit register
    fn register8(&self, register: Register8) -> u8 {
        match register {
            Register8::A => self.registers.a,
            Register8::B => self.registers.b,
            Register8::C => self.registers.c,
            Register8::D => self.registers.d,
            Register8::E => self.registers.e,
            Register8::H => self.registers.h,
            Register8::L => self.registers.l,
        }
    }

    /// Get a mutable reference to an 8-bit register
    fn register8_mut(&mut self, register: Register8) -> &mut u8 {
        match register {
            Register8::A => &mut self.registers.a,
            Register8::B => &mut self.registers.b,
            Register8::C => &mut self.registers.c,
            Register8::D => &mut self.registers.d,
            Register8::E => &mut self.registers.e,
            Register8::H => &mut self.registers.h,
            Register8::L => &mut self.registers.l,
        }
    }

    /// Get the value of a 16-bit register
    fn register16(&self, value: Register16) -> u16 {
        match value {
            Register16::Bc => self.registers.bc(),
            Register16::De => self.registers.de(),
            Register16::Hl => self.registers.hl(),
            Register16::Sp => self.registers.sp.0,
        }
    }

    /// Get a mutable reference to a 16-bit register
    fn register16_mut(&mut self, value: Register16) -> &mut u16 {
        match value {
            Register16::Bc => self.registers.bc_mut(),
            Register16::De => self.registers.de_mut(),
            Register16::Hl => self.registers.hl_mut(),
            Register16::Sp => &mut self.registers.sp.0,
        }
    }

    /// Get the value of a [Register16Memory]
    ///
    /// This is like [Self::Register16], but the available registers are
    /// slightly different. The `Hli` and `Hld` variants mutate the `HL`
    /// register *after* reporting its value.
    fn register16_mem(&mut self, register: Register16Memory) -> u16 {
        match register {
            Register16Memory::Bc => self.registers.bc(),
            Register16Memory::De => self.registers.de(),
            Register16Memory::Hli => {
                // This does NOT set flags
                let hl_mut = self.registers.hl_mut();
                let value = *hl_mut;
                *hl_mut = value.wrapping_add(1);
                value
            }
            Register16Memory::Hld => {
                let value = self.registers.hl();
                // This does NOT set flags
                *self.registers.hl_mut() = value.wrapping_sub(1);
                value
            }
        }
    }

    /// Get the byte of memory referenced by register HL
    fn hl_mem_mut(&mut self) -> Result<&mut u8, MemoryError> {
        self.memory.get8_mut(Address(self.registers.hl()))
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
    sp: Address,
    /// Program counter
    pc: Address,
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
#[derive(Copy, Clone, Debug, PartialEq)]
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
    Dec(DecInc),
    /// Disable interrupts
    Di,
    /// Enable interrupts
    Ei,
    /// Enter CPU low-power consumption mode until an interrupt occurs
    Halt,
    /// Increment a value by 1
    Inc(DecInc),
    /// Jump to another address in the code
    Jp(Jump),
    /// Jump a relative number of instructions in the code
    Jr {
        offset: i8,
        /// If defined, only jump when true
        condition: Option<ConditionCode>,
    },
    /// Move a value
    Ld(Load),
    /// Move a value, but different
    Ldh(LoadHigh),
    /// TODO
    Math { operation: Math, target: MathTarget },
    /// No op
    Nop,
    /// Push a 16-bit register value onto the stack
    Pop(Register16Stack),
    /// Pop a 16-bit value from the stack into a register
    Push(Register16Stack),
    /// Set a specific bit in a register to 0
    Res(Bit, Register8),
    /// Return from subroutine
    ///
    /// If the condition is defined, only return if it's true
    Ret(Option<ConditionCode>),
    /// Return from subroutine and enable interrupts
    Reti,
    /// Rotate a register left, through the carry flag
    Rl(Register8),
    /// Rotate register `a` left, through the carry flag
    Rla,
    /// Rotate a register left
    Rlc(Register8),
    /// Rotate register `a` left
    Rlca,
    /// Rotate a register right, through the carry flag
    Rr(Register8),
    /// Rotate register `a` right, through the carry flag
    Rra,
    /// Rotate a register right
    Rrc(Register8),
    /// Rotate register `a` right
    Rrca,
    /// Call a function at an address
    ///
    /// This is a faster alternative to `CALL` for addresses that can be packed
    /// into 3 bits. The translation to an address happens at parse time. This
    /// *could* be combined into [Self::Call], but keeping it separate makes
    /// debugging easier.
    Rst(Address),
    /// Set carry flag
    Scf,
    /// Set a specific bit in a register to 1
    Set(Bit, Register8),
    /// Shift left arithmetically a register
    Sla(Register8),
    /// Shift right arithmetically a register
    Sra(Register8),
    /// Shift right logically a register
    Srl(Register8),
    /// Swap the upper 4 bits of a register with the lower 4
    Swap(Register8),
    /// Enter CPU low power mode
    Stop,
    /// An invalid instruction from one of the 11 invalid opcodes
    Invalid,
}

/// TODO
#[derive(Copy, Clone, Debug, PartialEq)]
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
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum MathTarget {
    /// Byte in a register
    Register(Register8),
    /// Byte pointed to by register `hl`
    Hl,
    /// Constant value
    Const(u8),
}

/// Variations of the `ADD` instruction
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Add {
    /// Add a 16-bit value to `hl`
    Hl(Register16),
    /// Add `sp` to `hl`
    HlSp,
}

/// Variations of the `DEC` (decrement) and `INC` (increment) instructions
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum DecInc {
    /// Increment an 8-bit register
    R8(Register8),
    /// Increment a 16-bit register
    R16(Register16),
    /// Increment the byte pointed to by `hl`
    Hl,
}

/// Variations of the `JP` (jump) instruction
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Jump {
    /// Jump to a specific memory address
    Address(Address),
    /// Jump to a specific memory address if the condition is true
    AddressCc(ConditionCode, Address),
    /// Jump to the address in `hl`
    Hl,
}

/// Variations of the `LD` (load) instruction
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Load {
    /// Load from `a` to a memory address
    AddressA { dest: Address },
    /// Load from a memory address to `a`
    AAddress { source: Address },
    /// Load from `sp` to a memory address
    AddressSp { dest: Address },
    /// Add an offset to the value in `sp` and copy that into `hl`
    HlSpOffset { offset: i8 },
    /// Load from `sp` to `hl`
    SpHl,
    /// Load a constant into an 8-bit register
    R8Const { dest: Register8, source: u8 },
    /// Load from one 8-bit register to another
    R8R8 { dest: Register8, source: Register8 },
    /// Load a constant into a 16-bit register
    R16Const { dest: Register16, source: u16 },
    /// Load from register `a` to the byte pointed to by [Register16Memory]
    R16MemA { dest: Register16Memory },
    /// Load from the byte pointed to by [Register16Memory] into register `a`
    AR16Mem { source: Register16Memory },
}

/// Variations of the `LDH` (load high) instruction
///
/// This moves values in/out of the `$FF00-$FFFF` space of memory.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum LoadHigh {
    /// Copy the byte at address `$FF00+c` into register `a`
    AC,
    /// Copy the byte at `$FF00+offset` into register `a`
    AConst(u8),
    /// Copy the value in register `a` into the byte at address `$FF00+c`
    CA,
    /// Copy the value in register `a` into the byte at address `$FF00+offset`
    ConstA(u8),
}

/// Source of an 8-bit value
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Value8 {
    /// Value from a register
    Register(Register8),
    /// Constant value
    Const(u8),
    // TODO should this include [HL]?
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
/// `r8` on https://gbdev.io/pandocs/CPU_Instruction_Set.html EXCEPT this does
/// not include the `hl` variant. Every instruction that needs that instead
/// handles it separately. The behavior of that variant is different because it
/// includes a memory lookup.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Register8 {
    A,
    B,
    C,
    D,
    E,
    H,
    L,
}

/// Name of a 16-bit register
///
/// `r16` on https://gbdev.io/pandocs/CPU_Instruction_Set.html
#[derive(Copy, Clone, Debug, PartialEq)]
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

/// Name of a general purpose 16-bit register for stack operations
///
/// Most instructions use [Register16], but `PUSH`/`POP` use `af` instead of
/// `sp`
///
/// `r16stk` on https://gbdev.io/pandocs/CPU_Instruction_Set.html
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Register16Stack {
    /// Value in register `af`
    Af,
    /// Value in register `bc`
    Bc,
    /// Value in register `de`
    De,
    /// Value in register `hl`
    Hl,
}

/// 16-bit register for load operations
///
/// Most instructions use [Register16], but `LD` uses `hli` and `hld` (AKA `hl+`
/// and `hl-`). `r16mem` on https://gbdev.io/pandocs/CPU_Instruction_Set.html
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Register16Memory {
    /// Value in register `bc`
    Bc,
    /// Value in register `de`
    De,
    /// Read from/write to register `hl`, then increment it
    Hli,
    /// Read from/write to register `hl`, then decrement it
    Hld,
}

/// Condition for a conditional jump or call
#[derive(Copy, Clone, Debug, PartialEq)]
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
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Bit(pub u8);

/// Address of a byte of memory
///
/// https://rylev.github.io/DMG-01/public/book/memory_map.html
#[derive(Copy, Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct Address(pub u16);

impl Display for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const ADDRESS_WIDTH: usize = 4;
        write!(f, "0x{:0>ADDRESS_WIDTH$x}", self.0)
    }
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
