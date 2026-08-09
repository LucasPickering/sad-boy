//! CPU registers and instruction handling

mod math;

use crate::{
    emu::{
        Clock,
        clock::Cycles,
        instruction::{
            Add, ConditionCode, DecInc, Instruction, Jump, Load, LoadHigh,
            Operand, Register8, Register16, Register16Memory, Register16Stack,
            Value8,
        },
        memory::{self, Address, MemoryBus},
    },
    util::{Bit, BitPack, PackedBits, impl_bit_pack},
};
use std::{
    fmt::{self, Debug, Display},
    ops::{BitAnd, BitOr, BitXor},
};
use tracing::{error, info_span, trace};

/// Central Processing Unit for a Game Boy
///
/// This holds the CPU registers and executes instructions.
#[derive(Debug, Default, PartialEq)]
pub struct Cpu {
    /// Mutable values directly in the CPU
    registers: Registers,
    /// IME flag
    interrupts_enabled: bool,
    /// Most recently executed instruction
    ///
    /// Also includes the number of cycles it took and the size of the
    /// instruction in bytes.
    ///
    /// `None` only on startup.
    previous_instruction: Option<(Instruction, Cycles, usize)>,
}

impl Cpu {
    /// Execute the next CPU instruction
    ///
    /// This will take a variable number of CPU cycles based on the instruction
    /// executed.
    pub async fn execute_next(
        &mut self,
        clock: &Clock,
        mut memory: MemoryBus<'_>,
    ) {
        // Parse the next instruction and check how many cycles it will take
        let pc = self.registers.pc;
        let (instruction, num_bytes) = memory.get_instruction(pc);
        let cycles = self.exe(&mut memory).cycles(instruction);

        // Wait *before* executing so state isn't updated until after the
        // elapsed cycles
        clock.wait(cycles).await;
        self.exe(&mut memory).execute(instruction);

        // If the instruction didn't modify the PC (e.g. jumps), then
        // advance it automatically
        if self.registers.pc == pc {
            self.registers.pc.0 += num_bytes as u16;
        }

        self.previous_instruction = Some((instruction, cycles, num_bytes));
    }

    /// Get debug info about the CPU
    ///
    /// This is a read-only summary of current CPU state for the debugger. This
    /// needs the memory bus to load the next instruction from memory. That
    /// reference is `mut` so it can cache the instruction if needed.
    pub fn debug_info(&mut self, memory: &mut MemoryBus) -> CpuDebugInfo {
        // Parse the next instruction from memory. This seems duplicative, but
        // it will be cached within the memory bus so it won't be parsed during
        // the next execution.
        let (instruction, num_bytes) =
            memory.get_instruction(self.registers.pc);
        let cycles = self.exe(memory).cycles(instruction);

        let reg = &self.registers;
        CpuDebugInfo {
            a: reg.a,
            f: reg.f,
            af: reg.af(),
            b: reg.b,
            c: reg.c,
            bc: reg.bc(),
            d: reg.d,
            e: reg.e,
            de: reg.de(),
            h: reg.h,
            l: reg.l,
            hl: reg.hl(),
            pc: reg.pc,
            sp: reg.sp,
            interrupts_enabled: self.interrupts_enabled,
            previous_instruction: self.previous_instruction,
            next_instruction: (instruction, cycles, num_bytes),
        }
    }

    fn exe<'cpu, 'mem>(
        &'cpu mut self,
        memory: &'cpu mut MemoryBus<'mem>,
    ) -> CpuExe<'cpu, 'mem> {
        CpuExe {
            registers: &mut self.registers,
            interrupts_enabled: &mut self.interrupts_enabled,
            memory,
        }
    }
}

/// Exposed debug state for the CPU
pub struct CpuDebugInfo {
    // ===== Registers ====
    pub a: u8,
    pub f: PackedBits<BcdFlags>,
    pub af: u16,
    pub b: u8,
    pub c: u8,
    pub bc: u16,
    pub d: u8,
    pub e: u8,
    pub de: u16,
    pub h: u8,
    pub l: u8,
    pub hl: u16,
    pub pc: Address,
    pub sp: Address,

    /// Interrupt enable flag
    pub interrupts_enabled: bool,
    /// Most recently executed instruction
    ///
    /// Also includes the number of cycles it took and the size of the
    /// instruction in bytes.
    ///
    /// `None` only on startup.
    pub previous_instruction: Option<(Instruction, Cycles, usize)>,
    /// Next instruction to execute
    ///
    /// Also includes the number of cycles it will take and the size of the
    /// instruction in bytes.
    pub next_instruction: (Instruction, Cycles, usize),
}

impl Default for CpuDebugInfo {
    fn default() -> Self {
        Self {
            a: 0,
            f: 0.into(),
            af: 0,
            b: 0,
            c: 0,
            bc: 0,
            d: 0,
            e: 0,
            de: 0,
            h: 0,
            l: 0,
            hl: 0,
            pc: Address(0),
            sp: Address(0),
            interrupts_enabled: false,
            previous_instruction: None,
            next_instruction: (Instruction::Nop, Cycles(0), 0),
        }
    }
}

/// Helper for executing CPU instructions
///
/// This wraps all state together so it can be accessed easily by all execution
/// functions.
struct CpuExe<'cpu, 'mem> {
    registers: &'cpu mut Registers,
    interrupts_enabled: &'cpu mut bool,
    memory: &'cpu mut MemoryBus<'mem>,
}

impl CpuExe<'_, '_> {
    /// Execute a single CPU instruction
    fn execute(&mut self, instruction: Instruction) {
        let _span = info_span!("Instruction", %instruction).entered();
        trace!(registers = ?self.registers, "Executing instruction");
        match instruction {
            Instruction::Adc(rhs) => self.add_carry(rhs),
            Instruction::Add(add) => self.add(add),
            Instruction::And(rhs) => self.bit_binary(u8::bitand, rhs, true),
            Instruction::Bit(bit, source) => self.bit_get(bit, source),
            Instruction::Call { address, condition } => {
                // CALL instruction is 3 bytes
                self.call(3, address, condition);
            }
            Instruction::Ccf => {
                let flags = self.registers.flags();
                self.registers.set_flags(BcdFlags {
                    subtract: false,
                    half_carry: false,
                    carry: !flags.carry,
                    ..flags
                });
            }
            Instruction::Daa => self.daa(),
            Instruction::Cp(rhs) => self.compare(rhs),
            Instruction::Cpl => {
                self.registers.a = !self.registers.a;
                let flags = self.registers.flags();
                self.registers.set_flags(BcdFlags {
                    subtract: true,
                    half_carry: true,
                    ..flags
                });
            }
            Instruction::Dec(dec_inc) => self.dec_inc(dec_inc, true),
            Instruction::Di => *self.interrupts_enabled = false,
            Instruction::Ei => *self.interrupts_enabled = true,
            Instruction::Halt => todo!("HALT"),
            Instruction::Inc(dec_inc) => self.dec_inc(dec_inc, false),
            Instruction::Jp(jump) => self.jump(jump),
            Instruction::Jr { offset, condition } => {
                self.jump_relative(offset, condition);
            }
            Instruction::Ld(load) => self.load(load),
            Instruction::Ldh(load) => self.load_high(load),
            Instruction::Nop => {}
            Instruction::Or(rhs) => self.bit_binary(u8::bitor, rhs, false),
            Instruction::Push(register) => {
                let value = *self.register16_stack_mut(register);
                self.push(value);
            }
            Instruction::Pop(register) => {
                *self.register16_stack_mut(register) = self.pop();
            }
            Instruction::Res(bit, dest) => self.bit_set(bit, dest, false),
            Instruction::Ret(condition) => self.ret(condition),
            Instruction::Reti => {
                self.ret(None);
                *self.interrupts_enabled = true;
            }
            Instruction::Rl(dest) => self.bit_unary(
                |value, carry| {
                    (Bit(0).set(value.rotate_left(1), carry), Bit(7).get(value))
                },
                dest,
            ),
            Instruction::Rla => {
                let carry = self.registers.flags().carry;
                let old = self.registers.a;
                self.registers.a = Bit(0).set(old.rotate_left(1), carry);
                self.registers.set_flags(BcdFlags {
                    zero: false,
                    subtract: false,
                    half_carry: false,
                    carry: Bit(7).get(old),
                });
            }
            Instruction::Rlc(dest) => self.bit_unary(
                |value, _| (value.rotate_left(1), Bit(7).get(value)),
                dest,
            ),
            Instruction::Rlca => {
                let old = self.registers.a;
                self.registers.a = old.rotate_left(1);
                self.registers.set_flags(BcdFlags {
                    zero: false,
                    subtract: false,
                    half_carry: false,
                    carry: Bit(7).get(old),
                });
            }
            Instruction::Rr(dest) => self.bit_unary(
                |value, carry| {
                    (
                        Bit(7).set(value.rotate_right(1), carry),
                        Bit(0).get(value),
                    )
                },
                dest,
            ),
            Instruction::Rra => {
                let carry = self.registers.flags().carry;
                let old = self.registers.a;
                self.registers.a = Bit(7).set(old.rotate_right(1), carry);
                self.registers.set_flags(BcdFlags {
                    zero: false,
                    subtract: false,
                    half_carry: false,
                    carry: Bit(0).get(old),
                });
            }
            Instruction::Rrc(dest) => self.bit_unary(
                |value, _| (value.rotate_right(1), Bit(0).get(value)),
                dest,
            ),
            Instruction::Rrca => {
                let old = self.registers.a;
                self.registers.a = old.rotate_right(1);
                self.registers.set_flags(BcdFlags {
                    zero: false,
                    subtract: false,
                    half_carry: false,
                    carry: Bit(0).get(old),
                });
            }
            Instruction::Rst(address) => self.call(1, address, None),
            Instruction::Sbc(rhs) => self.subtract_carry(rhs),
            Instruction::Scf => {
                let flags = self.registers.flags();
                self.registers.set_flags(BcdFlags {
                    subtract: false,
                    half_carry: false,
                    carry: true,
                    ..flags
                });
            }
            Instruction::Set(bit, dest) => self.bit_set(bit, dest, true),
            Instruction::Sla(dest) => {
                self.bit_unary(
                    |value, _| (value << 1, Bit(7).get(value)),
                    dest,
                );
            }
            Instruction::Sra(dest) => self.bit_unary(
                |value, _| {
                    // Don't modify bit 7
                    (
                        Bit(7).set(value >> 1, Bit(7).get(value)),
                        Bit(0).get(value),
                    )
                },
                dest,
            ),
            Instruction::Srl(dest) => {
                self.bit_unary(
                    |value, _| (value >> 1, Bit(0).get(value)),
                    dest,
                );
            }
            // STOP is hard
            // https://gbdev.io/pandocs/Reducing_Power_Consumption.html
            Instruction::Stop => unimplemented!("STOP"),
            Instruction::Sub(rhs) => self.subtract(rhs),
            Instruction::Swap(dest) => {
                self.bit_unary(|value, _| (value.rotate_right(4), false), dest);
            }
            Instruction::Xor(rhs) => self.bit_binary(u8::bitxor, rhs, false),
            Instruction::Invalid => error!("Invalid instruction"),
        }
    }

    /// How many CPU cycles will this instruction take to execute?
    ///
    /// TODO note about dynamic instructions
    fn cycles(&self, instruction: Instruction) -> Cycles {
        let cycles = match instruction {
            Instruction::Adc(operand)
            | Instruction::Add(Add::A(operand))
            | Instruction::And(operand)
            | Instruction::Cp(operand)
            | Instruction::Or(operand)
            | Instruction::Sbc(operand)
            | Instruction::Sub(operand)
            | Instruction::Xor(operand) => match operand {
                Operand::V8(Value8::Register(_)) => 1,
                Operand::V8(Value8::Hl) | Operand::Const(_) => 2,
            },
            Instruction::Add(Add::Hl(_)) => 2,
            Instruction::Add(Add::Sp(_)) => 4,
            Instruction::Bit(_, value) => match value {
                Value8::Register(_) => 2,
                Value8::Hl => 3,
            },
            Instruction::Call {
                condition: Some(cond),
                ..
            } if !self.condition(cond) => 3, // Call is NOT made
            Instruction::Call { .. } => 6, // Call is made
            Instruction::Ccf => 1,
            Instruction::Cpl => 1,
            Instruction::Daa => 1,
            Instruction::Dec(dec_inc) | Instruction::Inc(dec_inc) => {
                match dec_inc {
                    DecInc::V8(Value8::Register(_)) => 1,
                    DecInc::V8(Value8::Hl) => 3,
                    DecInc::R16(_) => 2,
                }
            }
            Instruction::Di => 1,
            Instruction::Ei => 1,
            Instruction::Halt | Instruction::Stop => todo!(),
            Instruction::Jp(jump) => match jump {
                // Jump NOT taken
                Jump::AddressCc(cond, _) if !self.condition(cond) => 3,
                // Jump taken
                Jump::Address(_) | Jump::AddressCc(_, _) => 4,
                Jump::Hl => 1,
            },
            Instruction::Jr {
                condition: Some(cond),
                ..
            } if !self.condition(cond) => 2, // Jump NOT taken
            Instruction::Jr { .. } => 3, // Jump taken
            Instruction::Ld(load) => match load {
                Load::AddressA { .. } | Load::AAddress { .. } => 4,
                Load::AddressSp { .. } => 5,
                Load::HlSpOffset { .. } => 3,
                Load::SpHl => 2,
                Load::V8Const {
                    dest: Value8::Register(_),
                    ..
                } => 2,
                Load::V8Const {
                    dest: Value8::Hl, ..
                } => 3,
                Load::V8V8 {
                    dest: Value8::Register(_),
                    source: Value8::Register(_),
                } => 1,
                Load::V8V8 {
                    dest: Value8::Hl,
                    source: Value8::Register(_),
                }
                | Load::V8V8 {
                    dest: Value8::Register(_),
                    source: Value8::Hl,
                } => 2,
                Load::V8V8 {
                    dest: Value8::Hl,
                    source: Value8::Hl,
                } => unreachable!("LD [HL],[HL] should parse as HALT"),
                Load::R16Const { .. } => 3,
                Load::R16MemA { .. } | Load::AR16Mem { .. } => 2,
            },
            Instruction::Ldh(load) => match load {
                LoadHigh::AC => 2,
                LoadHigh::AConst(_) => 3,
                LoadHigh::CA => 2,
                LoadHigh::ConstA(_) => 3,
            },
            Instruction::Nop => 1,
            Instruction::Pop(_) => 3,
            Instruction::Push(_) => 4,
            Instruction::Res(_, value)
            | Instruction::Set(_, value)
            | Instruction::Rl(value)
            | Instruction::Rlc(value)
            | Instruction::Rr(value)
            | Instruction::Rrc(value)
            | Instruction::Sla(value)
            | Instruction::Sra(value)
            | Instruction::Srl(value)
            | Instruction::Swap(value) => match value {
                Value8::Register(_) => 2,
                Value8::Hl => 4,
            },
            Instruction::Ret(None) | Instruction::Reti => 4,
            Instruction::Ret(Some(cond)) if self.condition(cond) => 5,
            Instruction::Ret(Some(_)) => 2,
            Instruction::Rla
            | Instruction::Rlca
            | Instruction::Rra
            | Instruction::Rrca => 1,
            Instruction::Rst(_) => 4,
            Instruction::Scf => 1,
            Instruction::Invalid => 0,
        };
        Cycles(cycles)
    }

    /// Execute a function call
    fn call(
        &mut self,
        instruction_size: u16,
        target: Address,
        condition: Option<ConditionCode>,
    ) {
        if condition.is_none_or(|cond| self.condition(cond)) {
            // Push the return address, which is the instruction *after* the
            // CALL/RST
            self.push(self.registers.pc.0 + instruction_size);
            self.registers.pc = target;
        }
    }

    /// Execute a `JP` instruction
    fn jump(&mut self, jump: Jump) {
        match jump {
            Jump::Address(address) => self.registers.pc = address,
            Jump::AddressCc(condition, address) => {
                if self.condition(condition) {
                    self.registers.pc = address;
                }
            }
            Jump::Hl => self.registers.pc = Address(self.registers.hl()),
        }
    }

    /// Execute a `JR` instruction
    fn jump_relative(&mut self, offset: i8, condition: Option<ConditionCode>) {
        if condition.is_none_or(|cond| self.condition(cond)) {
            // Offset is relative to the instruction *after* the jump, and this
            // instruction is always 2 bytes
            let bytes = 2;
            self.registers.pc = Address(
                self.registers.pc.0.strict_add_signed(offset.into()) + bytes,
            );
        }
    }

    /// Execute an `LD` instruction
    fn load(&mut self, load: Load) {
        match load {
            Load::AddressA { dest } => self.memory.set8(dest, self.registers.a),
            Load::AAddress { source } => {
                self.registers.a = self.memory.get8(source);
            }
            Load::AddressSp { dest } => {
                self.memory.set16(dest, self.registers.sp.0);
            }
            Load::HlSpOffset { offset } => {
                let lhs = self.registers.sp.0;
                let (value, carry) = lhs.overflowing_add_signed(offset.into());
                *self.registers.hl_mut() = value;
                self.registers.set_flags(BcdFlags {
                    zero: false,
                    subtract: false,
                    half_carry: math::half_carry16(lhs, offset as u16, value),
                    carry,
                });
            }
            Load::SpHl => self.registers.sp = Address(self.registers.hl()),
            // LD r8,n8
            Load::V8Const {
                dest: Value8::Register(dest),
                source,
            } => *self.register8_mut(dest) = source,
            // LD [HL],n8
            Load::V8Const {
                dest: Value8::Hl,
                source,
            } => self.set_hl_mem(source),
            // LD r8,r8
            Load::V8V8 {
                dest: Value8::Register(dest),
                source: Value8::Register(source),
            } => *self.register8_mut(dest) = self.register8(source),
            // LD [HL],r8
            Load::V8V8 {
                dest: Value8::Hl,
                source: Value8::Register(source),
            } => self.set_hl_mem(self.register8(source)),
            // LD r8,[HL]
            Load::V8V8 {
                dest: Value8::Register(dest),
                source: Value8::Hl,
            } => *self.register8_mut(dest) = self.hl_mem(),
            // LD [HL],[HL] is not valid - that's the opcode for HALT
            Load::V8V8 {
                dest: Value8::Hl,
                source: Value8::Hl,
            } => unreachable!("LD [HL],[HL] should parse as HALT"),
            Load::R16Const { dest, source } => {
                *self.register16_mut(dest) = source;
            }
            Load::R16MemA { dest } => {
                let dest = Address(self.register16_mem(dest));
                self.memory.set8(dest, self.registers.a);
            }
            Load::AR16Mem { source } => {
                let source = Address(self.register16_mem(source));
                self.registers.a = self.memory.get8(source);
            }
        }
    }

    /// Execute an `LDH` instruction
    fn load_high(&mut self, load: LoadHigh) {
        fn addr(low: u8) -> Address {
            Address(0xFF00 + u16::from(low))
        }

        match load {
            LoadHigh::AC => {
                self.registers.a = self.memory.get8(addr(self.registers.c));
            }
            LoadHigh::AConst(source) => {
                self.registers.a = self.memory.get8(addr(source));
            }
            LoadHigh::CA => {
                self.memory.set8(addr(self.registers.c), self.registers.a);
            }
            LoadHigh::ConstA(dest) => {
                self.memory.set8(addr(dest), self.registers.a);
            }
        }
    }

    /// Push a 16-bit value onto the stack
    fn push(&mut self, value: u16) {
        // SP points to the LAST OCCUPIED slot, so we have to move it back
        // BEFORE writing
        self.registers.sp.0 -= 2;
        debug_assert!(
            memory::HIGH_RAM.contains(self.registers.sp),
            "Stack pointer {} is outside range {}",
            self.registers.sp,
            memory::HIGH_RAM
        );
        self.memory.set16(self.registers.sp, value);
    }

    /// Pop a 16-bit value from the top of the stack
    fn pop(&mut self) -> u16 {
        let value = self.memory.get16(self.registers.sp);
        // SP points to the LAST OCCUPIED slot, so we need to increment it to
        // "deallocate" the value we just popped.
        self.registers.sp.0 += 2;
        debug_assert!(
            memory::HIGH_RAM.contains(self.registers.sp),
            "Stack pointer {} is outside range {}",
            self.registers.sp,
            memory::HIGH_RAM
        );

        value
    }

    /// Return from the current function
    fn ret(&mut self, condition: Option<ConditionCode>) {
        match condition {
            Some(cond) if self.condition(cond) => {
                self.registers.pc = Address(self.pop());
            }
            Some(_) => {} // Condition false
            None => {
                self.registers.pc = Address(self.pop());
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
const _: () = assert!(
    cfg!(target_endian = "little"),
    "Only little-endian platforms are supported (for register pairs)"
);

/// Registers in a Game Boy CPU
#[derive(Default, PartialEq)]
#[repr(C)] // Field ordering/alignment is important
struct Registers {
    // Registers are ordered so pairs are kept together. This allows them to be
    // accessed as separate bytes or a pair together. The pairs are SWAPPED
    // here because `af` means `a` is the high byte and `f` is the low byte.
    // The assertion above ensures we're on an little-endian system.

    // af
    f: PackedBits<BcdFlags>,
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

    /// Program counter
    pc: Address,
    /// Stack pointer
    ///
    /// The stack is a series of 16-bit values at the high end of working RAM.
    /// The bottom value of the stack will be the final value of RAM, and the
    /// stack grows backward from there. This points to the *last occupied slot
    /// on the stack*, meaning the SP must be decremented *before* pushing
    /// and incremented *after* popping.
    sp: Address,
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

/// Generate methods on [Registers] to access two registers as a 16-bit value
///
/// The methods use unsafe operations to treat the two registers as a single
/// value. For that reason, **field order on [Registers] is extremely
/// important.** The pointer to the first register of the pair is case from a
/// `u8` pointer to a `u16` pointer; the second register is **assumed** to
/// be the following byte in memory.
macro_rules! register_pair {
    // External branch
    ($r_high:ident, $r_low:ident) => {
        // Generate identifiers and defer to the next branch
        paste::paste! {
            register_pair!([<$r_high $r_low>], [<$r_high $r_low _mut>], $r_low);
        }
    };
    // Internal branch
    ($pair:ident, $pair_mut:ident, $r_low:ident) => {
        /// Get the value of the `$pair` register pair
        fn $pair(&self) -> u16 {
            // SAFETY: Safety is predicated on the macro being called with
            // registers that are paired together in the struct layout.
            // - Alignment is safe because u16 is 2-byte aligned and the
            //   registers are pairs of 2. The entire struct is aligned, so
            //   every other register (i.e. the lower register of each pair)
            //   will be 2-byte aligned
            // - This will not read/write out of bounds because the first
            //   register must have a second register after it.
            let ptr8 = std::ptr::from_ref(&self.$r_low);
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
            let ptr8 = std::ptr::from_mut(&mut self.$r_low);
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
    register_pair!(a, f);
    register_pair!(b, c);
    register_pair!(d, e);
    register_pair!(h, l);

    /// Read bit flags from the `f` register
    fn flags(&self) -> BcdFlags {
        self.f.unpack()
    }

    /// Set the `f` register to the given flags
    fn set_flags(&mut self, flags: BcdFlags) {
        self.f = flags.pack();
    }
}

/// The `f` register holds a set of 4 flags providing feedback about the
/// previous operation, for use with Binary Coded Decimal values
///
/// Use [Registers::flags] to get this value.
///
/// https://gbdev.io/pandocs/CPU_Registers_and_Flags.html#the-flags-register-lower-8-bits-of-af-register
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BcdFlags {
    /// Was the result of the operation zero?
    zero: bool,
    /// Was the operation a subtraction?
    subtract: bool,
    /// Did the result overflow from bit 3 (bit 7 for 16-bit ops)?
    half_carry: bool,
    /// Did the result overflow the value and wrap?
    carry: bool,
}

impl Display for BcdFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn b(b: bool) -> &'static str {
            if b { "1" } else { "0" }
        }
        write!(
            f,
            "z={},n={},h={},c={}",
            b(self.zero),
            b(self.subtract),
            b(self.half_carry),
            b(self.carry)
        )
    }
}

impl_bit_pack! {
    struct BcdFlags;
    Bit(7).mask() => zero,
    Bit(6).mask() => subtract,
    Bit(5).mask() => half_carry,
    Bit(4).mask() => carry,
}

/// Expected CPU state when exiting the bootloader
///
/// This is defined here so it can access private types/fields, but exported so
/// it can be used in the emulator-level test. I think this is better than
/// exposing a bunch of internals.
#[cfg(test)]
pub static BOOTLOADER_EXPECTED: Cpu = Cpu {
    registers: Registers {
        a: 1,
        f: PackedBits::new(0b0000_0000),
        b: 0,
        c: 19,
        // The final bootloader routine is to compare the Nintendo logo in the
        // bootloader to the one in the cartridge. DE points to the logo in the
        // the bootloader, HL in the cart.
        //
        // Starts at $00A8, +$30 for the logo comparison
        d: 0x00,
        e: 0xD8,
        // Starts at $0104, +$30 for the logo comparison, +$19 for the checksum
        h: 0x01,
        l: 0x4D,
        sp: Address(0xFFFE), // Stack is empty
        pc: Address(0x0100), // First instruction of the ROM
    },
    // https://gbdev.io/pandocs/Interrupts.html#ime-interrupt-master-enable-flag-write-only
    interrupts_enabled: false,
    previous_instruction: Some((
        Instruction::Ldh(LoadHigh::ConstA(0x50)),
        Cycles(3),
        2,
    )),
};

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

    fn zero(register: BcdFlags) -> bool {
        register.zero
    }

    fn subtract(register: BcdFlags) -> bool {
        register.subtract
    }

    fn half_carry(register: BcdFlags) -> bool {
        register.half_carry
    }

    fn carry(register: BcdFlags) -> bool {
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
        #[case] getter: impl FnOnce(BcdFlags) -> bool,
        #[case] register_value: u8,
        #[case] expected: bool,
    ) {
        let registers = Registers {
            f: PackedBits::new(register_value),
            ..Default::default()
        };
        let actual = getter(registers.flags());
        assert_eq!(actual, expected);
    }
}
