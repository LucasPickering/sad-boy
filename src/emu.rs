//! Emulation logic for a Nintendo Game Boy
//!
//! https://rylev.github.io/DMG-01/public/book/introduction.html

mod math;

use crate::{
    instruction::{
        Address, ConditionCode, Instruction, Jump, Load, Register8, Register16,
        Register16Memory, Register16Stack, Value8,
    },
    memory::{self, MemoryMap},
    rom::Rom,
};
use color_eyre::eyre;
use static_assertions::assert_cfg;
use std::{
    fmt::{self, Debug},
    ops::{BitAnd, BitOr, BitXor},
    path::Path,
    thread,
    time::{Duration, Instant},
};
use tracing::{error, info_span, trace};

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

    /// Create a [GameBoy] with an empty ROM for testing
    #[cfg(test)]
    fn empty() -> Self {
        Self {
            registers: Registers::default(),
            memory: MemoryMap::new(Rom::empty()),
        }
    }

    /// Keep running until the CPU is halted
    pub fn run(&mut self) {
        /// TODO explain
        const CYCLES_PER_FRAME: usize = 70224;
        let frame_time = Duration::from_secs_f64(1.0 / 60.0);

        loop {
            // https://josaphat.co/posts/gameboy-emulator/
            let mut cycle_budget = CYCLES_PER_FRAME;
            let frame_start = Instant::now();

            while cycle_budget > 0 {
                let (instruction, num_bytes) =
                    self.memory.get_instruction(self.registers.pc);
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
        let _span = info_span!(
            "Instruction",
            ?instruction,
            registers = ?self.registers,
        )
        .entered();
        trace!("Executing");
        match instruction {
            Instruction::Adc(rhs) => self.adc(rhs),
            Instruction::Add(add) => self.add(add),
            Instruction::And(rhs) => self.bitwise(u8::bitand, rhs, true),
            Instruction::Call { address, condition } => {
                self.call(address, condition)
            }
            Instruction::Dec(dec_inc) => self.dec_inc(dec_inc, -1),
            Instruction::Inc(dec_inc) => self.dec_inc(dec_inc, 1),
            Instruction::Jp(jump) => self.jump(jump),
            Instruction::Ld(load) => self.load(load),
            Instruction::Nop => 1,
            Instruction::Or(rhs) => self.bitwise(u8::bitor, rhs, false),
            Instruction::Push(register) => {
                let value = *self.register16_stack_mut(register);
                self.push(value);
                4
            }
            Instruction::Pop(register) => {
                *self.register16_stack_mut(register) = self.pop();
                3
            }
            Instruction::Ret(condition) => self.ret(condition),
            Instruction::Reti => {
                self.ret(None);
                // TODO enable interrupts
                4
            }
            Instruction::Sub(rhs) => self.subtract(rhs),
            Instruction::Xor(rhs) => self.bitwise(u8::bitxor, rhs, false),
            _ => {
                error!("Unknown instruction");
                1
            }
        }
    }

    /// Execute a function call
    ///
    /// Return the number of consumed CPU cycles
    fn call(
        &mut self,
        address: Address,
        condition: Option<ConditionCode>,
    ) -> usize {
        if condition.is_none_or(|cond| self.condition(cond)) {
            // Push the address of the instruction *after* this one
            self.push(self.registers.pc.next().0);
            self.registers.pc = address;
            6
        } else {
            3 // Quick exit
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

    /// Push a 16-bit value onto the stack
    fn push(&mut self, value: u16) {
        // SP points to the LAST OCCUPIED slot, so we have to move it back
        // BEFORE writing
        self.registers.sp.0 -= 2;
        debug_assert!(
            memory::RAM.contains(self.registers.sp),
            "Stack pointer {} is outside range {}",
            self.registers.sp,
            memory::RAM
        );
        self.memory.set16(self.registers.sp, value);
    }

    /// Pop a 16-bit value from the top of the stack
    fn pop(&mut self) -> u16 {
        // TODO make sure the stack isn't empty
        let value = self.memory.get16(self.registers.sp);
        // SP points to the LAST OCCUPIED slot, so we need to increment it to
        // "deallocate" the value we just popped.
        self.registers.sp.0 += 2;
        debug_assert!(
            memory::RAM.contains(self.registers.sp),
            "Stack pointer {} is outside range {}",
            self.registers.sp,
            memory::RAM
        );

        value
    }

    /// Return from the current function
    ///
    /// Return the number of consumed CPU cycles.
    fn ret(&mut self, condition: Option<ConditionCode>) -> usize {
        match condition {
            Some(cond) if self.condition(cond) => {
                self.registers.pc = Address(self.pop());
                5
            }
            Some(_) => 2, // Condition false
            None => {
                self.registers.pc = Address(self.pop());
                4
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

    /// Get the value of an 8-bit register
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

    /// Get a mutable reference to a [Register16Stack]
    fn register16_stack_mut(&mut self, register: Register16Stack) -> &mut u16 {
        match register {
            Register16Stack::Bc => self.registers.bc_mut(),
            Register16Stack::De => self.registers.de_mut(),
            Register16Stack::Hl => self.registers.hl_mut(),
            Register16Stack::Af => self.registers.af_mut(),
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

// Optimizations below rely on this.
assert_cfg!(target_endian = "little");

/// Registers in a Game Boy CPU
#[repr(C)] // Field ordering/alignment is important
struct Registers {
    // Registers are ordered so pairs are kept together. This allows them to be
    // accessed as separate bytes or a pair together. The pairs are SWAPPED
    // here because `af` means `a` is the high byte and `f` is the low byte.
    // The assertion above ensures we're on an little-endian system.

    // af
    f: u8,
    a: u8,
    // bc
    c: u8,
    b: u8,
    // de
    e: u8,
    d: u8,
    // hl
    l: u8,
    h: u8,

    /// Stack pointer
    ///
    /// The stack is a series of 16-bit values at the high end of working RAM.
    /// The bottom value of the stack will be the final value of RAM, and the
    /// stack grows backward from there. This points to the *last occupied slot
    /// on the stack*, meaning the SP must be decremented *before* pushing
    /// and incremented *after* popping.
    sp: Address,
    /// Program counter
    pc: Address,
}

impl Debug for Registers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Include virtual 16-bit register pairs in the output
        f.debug_struct("Registers")
            .field("a", &self.a)
            .field("f", &self.f)
            .field("af", &self.af())
            .field("b", &self.b)
            .field("c", &self.c)
            .field("bc", &self.bc())
            .field("d", &self.d)
            .field("e", &self.e)
            .field("de", &self.de())
            .field("h", &self.h)
            .field("l", &self.l)
            .field("hl", &self.hl())
            .field("sp", &self.sp)
            .field("pc", &self.pc)
            .finish()
    }
}

impl Default for Registers {
    fn default() -> Self {
        Self {
            f: 0,
            a: 0,
            c: 0,
            b: 0,
            e: 0,
            d: 0,
            l: 0,
            h: 0,
            // Stack starts at the end of RAM
            sp: Address(memory::RAM.end() + 1),
            // Skip the boot ROM, go straight to the game's ROM
            // https://gbdev.io/pandocs/Power_Up_Sequence.html
            pc: Address(0x0100),
        }
    }
}

/// Generate methods on [Registers] to access two registers as a 16-bit value
///
/// The methods use unsafe operations to treat the two registers as a single
/// value. For that reason, **field order on [Registers] is extremely
/// important.** The pointer to the first register of the pair is case from a
/// `u8` pointer to a `u16` pointer; the second register is **assumed** to
/// be the following byte in memory.
///
/// The `$r1` register should be the register with the *lower* bits. Because the
/// system is little-endian, that register must come first in memory.
macro_rules! register_pair {
    ($pair:ident, $pair_mut:ident, $r1:ident) => {
        /// Get the value of the `$pair` register pair
        fn $pair(&self) -> u16 {
            // SAFETY: Safety is predicated on the macro being called with
            // registers that are paired together in the struct layout.
            // - Alignment is safe because u16 is 2-byte aligned and the
            //   registers are pairs of 2. The entire struct is aligned, so
            //   every other register (i.e. the first register of each pair)
            //   will be 2-byte aligned
            // - This will not read/write out of bounds because the first
            //   register must have a second register after it.
            let ptr8 = std::ptr::from_ref(&self.$r1);
            debug_assert_eq!(
                ptr8.align_offset(2),
                0,
                "Register pointer must be 2-byte aligned"
            );
            #[expect(clippy::cast_ptr_alignment)]
            let ptr16 = ptr8.cast::<u16>();
            unsafe { *ptr16 }
        }

        /// Get a mutable reference to the `$pair` register pair
        fn $pair_mut(&mut self) -> &mut u16 {
            // SAFETY: see above fn
            let ptr8 = std::ptr::from_mut(&mut self.$r1);
            debug_assert_eq!(
                ptr8.align_offset(2),
                0,
                "Register pointer must be 2-byte aligned"
            );
            #[expect(clippy::cast_ptr_alignment)]
            let ptr16 = ptr8.cast::<u16>();
            unsafe { &mut *ptr16 }
        }
    };
}

impl Registers {
    register_pair!(af, af_mut, f);
    register_pair!(bc, bc_mut, c);
    register_pair!(de, de_mut, e);
    register_pair!(hl, hl_mut, l);

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
/// https://gbdev.io/pandocs/CPU_Registers_and_Flags.html#the-flags-register-lower-8-bits-of-af-register
#[derive(Copy, Clone, Debug, PartialEq)]
#[expect(clippy::struct_excessive_bools)]
struct Flags {
    /// Was the result of the operation zero?
    zero: bool,
    /// Was the operation a subtraction?
    subtract: bool,
    /// Did the result overflow from bit 3 (bit 7 for 16-bit ops)?
    half_carry: bool,
    /// Did the result overflow the value and wrap?
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

    // Static functions for test cases

    fn a(registers: &mut Registers) -> &mut u8 {
        &mut registers.a
    }

    fn f(registers: &mut Registers) -> &mut u8 {
        &mut registers.f
    }

    fn b(registers: &mut Registers) -> &mut u8 {
        &mut registers.b
    }

    fn c(registers: &mut Registers) -> &mut u8 {
        &mut registers.c
    }

    fn d(registers: &mut Registers) -> &mut u8 {
        &mut registers.d
    }

    fn e(registers: &mut Registers) -> &mut u8 {
        &mut registers.e
    }

    fn h(registers: &mut Registers) -> &mut u8 {
        &mut registers.h
    }

    fn l(registers: &mut Registers) -> &mut u8 {
        &mut registers.l
    }

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

    /// Test reading/writing all register pairs
    #[rstest]
    #[case::af(a, f, Registers::af, Registers::af_mut)]
    #[case::bc(b, c, Registers::bc, Registers::bc_mut)]
    #[case::de(d, e, Registers::de, Registers::de_mut)]
    #[case::hl(h, l, Registers::hl, Registers::hl_mut)]
    fn register_pairs(
        #[case] high: fn(&mut Registers) -> &mut u8,
        #[case] low: fn(&mut Registers) -> &mut u8,
        #[case] pair_read: fn(&Registers) -> u16,
        #[case] pair_write: fn(&mut Registers) -> &mut u16,
    ) {
        let mut registers = Registers::default();
        // Write individuals, read pair
        *high(&mut registers) = 0x12;
        *low(&mut registers) = 0x34;
        assert_eq!(pair_read(&registers), 0x1234);
        // Write to pair, read individual
        *pair_write(&mut registers) = 0xabcd;
        assert_eq!(*high(&mut registers), 0xab);
        assert_eq!(*low(&mut registers), 0xcd);
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
