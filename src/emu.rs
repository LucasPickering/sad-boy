//! Emulation logic for a Nintendo Game Boy
//!
//! https://rylev.github.io/DMG-01/public/book/introduction.html

use crate::{
    instruction::{
        Address, ConditionCode, DecInc, Instruction, Jump, Load, Register8,
        Register16, Register16Memory, Value8,
    },
    memory::MemoryMap,
    rom::Rom,
};
use color_eyre::eyre;
use std::{
    path::Path,
    thread,
    time::{Duration, Instant},
};
use tracing::{error_span, info_span, trace_span, warn};

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
                let cycles = self.execute(instruction);
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
    fn execute(&mut self, instruction: Instruction) -> usize {
        let _span = info_span!("Executing instruction", ?instruction).entered();
        match instruction {
            Instruction::Nop => 1,
            Instruction::Dec(dec_inc) => self.dec_inc(dec_inc, -1),
            Instruction::Inc(dec_inc) => self.dec_inc(dec_inc, 1),
            Instruction::Jp(jump) => self.jump(jump),
            Instruction::Ld(load) => self.load(load),
            _ => {
                warn!("Unknown instruction");
                1
            }
        }
    }

    /// Execute a `DEC` or `INC` instruction
    ///
    /// `delta` should be `-1` for `DEC`, `1` for `INC` Return the number of
    /// consumed CPU cycles.
    fn dec_inc(&mut self, dec_inc: DecInc, delta: i8) -> usize {
        // TODO set flags
        match dec_inc {
            DecInc::V8(Value8::Register(register)) => {
                let register = self.register8_mut(register);
                *register = register.wrapping_add_signed(delta);
                1
            }
            DecInc::V8(Value8::Hl) => {
                self.set_hl_mem(self.hl_mem().wrapping_add_signed(delta));
                3
            }
            DecInc::R16(register) => {
                let register = self.register16_mut(register);
                *register = register.wrapping_add_signed(delta.into());
                2
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
    fn load(&mut self, load: Load) -> usize {
        match load {
            Load::AddressA { dest } => {
                self.memory.set8(dest, self.registers.a);
                4
            }
            Load::AAddress { source } => {
                self.registers.a = self.memory.get8(source);
                4
            }
            Load::AddressSp { dest } => {
                self.memory.set16(dest, self.registers.sp.0);
                5
            }
            Load::HlSpOffset { offset } => {
                let value =
                    self.registers.sp.0.wrapping_add_signed(offset.into());
                *self.registers.hl_mut() = value;
                // TODO set flags here
                // https://rgbds.gbdev.io/docs/v1.0.1/gbz80.7#LD_HL,SP+e8
                3
            }
            Load::SpHl => {
                self.registers.sp = Address(self.registers.hl());
                2
            }
            // LD r8,n8
            Load::V8Const {
                dest: Value8::Register(dest),
                source,
            } => {
                *self.register8_mut(dest) = source;
                2
            }
            // LD [HL],n8
            Load::V8Const {
                dest: Value8::Hl,
                source,
            } => {
                self.set_hl_mem(source);
                2
            }
            // LD r8,r8
            Load::V8V8 {
                dest: Value8::Register(dest),
                source: Value8::Register(source),
            } => {
                *self.register8_mut(dest) = self.register8(source);
                1
            }
            // LD [HL],r8
            Load::V8V8 {
                dest: Value8::Hl,
                source: Value8::Register(source),
            } => {
                self.set_hl_mem(self.register8(source));
                2
            }
            // LD r8,[HL]
            Load::V8V8 {
                dest: Value8::Register(dest),
                source: Value8::Hl,
            } => {
                *self.register8_mut(dest) = self.hl_mem();
                2
            }
            // LD [HL],[HL] is not valid - that's the opcode for HALT
            Load::V8V8 {
                dest: Value8::Hl,
                source: Value8::Hl,
            } => unreachable!("LD [HL],[HL] should parse as HALT"),
            Load::R16Const { dest, source } => {
                *self.register16_mut(dest) = source;
                3
            }
            Load::R16MemA { dest } => {
                let dest = Address(self.register16_mem(dest));
                self.memory.set8(dest, self.registers.a);
                2
            }
            Load::AR16Mem { source } => {
                let source = Address(self.register16_mem(source));
                self.registers.a = self.memory.get8(source);
                2
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

    /// Get the byte of memory referenced by register `hl`
    fn hl_mem(&self) -> u8 {
        self.memory.get8(Address(self.registers.hl()))
    }

    /// Set the value of the byte of memory pointed to by register `hl`
    fn set_hl_mem(&mut self, value: u8) {
        self.memory.set8(Address(self.registers.hl()), value);
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
            #[expect(clippy::cast_ptr_alignment)]
            unsafe {
                *(&raw const self.$r1).cast::<u16>()
            }
        }

        /// Get a mutable reference to the `$pair` register pair
        fn $pair_mut(&mut self) -> &mut u16 {
            // SAFETY: TODO
            #[expect(clippy::cast_ptr_alignment)]
            unsafe {
                &mut *(&raw mut self.$r1).cast::<u16>()
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
