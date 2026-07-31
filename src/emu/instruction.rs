//! Game Boy CPU instructions

use crate::{emu::memory::Address, util::Bit};
use std::fmt::{self, Display};

/// CPU instruction
///
/// https://gbdev.io/pandocs/CPU_Instruction_Set.html
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Instruction {
    /// Add a value plus the carry flag to `a`
    Adc(Operand),
    /// Add a value to a register
    Add(Add),
    /// Bitwise AND between `a` and another value (modifies `a`)
    And(Operand),
    /// Get a single bit from a register (output to the `zero` flag)
    Bit(Bit, Value8),
    /// Push a new frame onto the stack, then set `pc` to that address
    Call {
        address: Address,
        /// If defined, only call if true
        condition: Option<ConditionCode>,
    },
    /// Complement (invert) carry flag
    Ccf,
    /// Compare register `a` with another value (modifies flags but NOT `a`)
    Cp(Operand),
    /// Complement (bitwise NOT) register `a`
    Cpl,
    /// Decimal Adjust Accumulator
    ///
    /// https://blog.ollien.com/posts/gb-daa/
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
    ///
    /// See [LoadHigh]
    Ldh(LoadHigh),
    /// No op
    Nop,
    /// Bitwise OR between `a` and another value (modifies `a`)
    Or(Operand),
    /// Push a 16-bit register value onto the stack
    Pop(Register16Stack),
    /// Pop a 16-bit value from the stack into a register
    Push(Register16Stack),
    /// Set a specific bit in a register to 0
    Res(Bit, Value8),
    /// Return from subroutine
    ///
    /// If the condition is defined, only return if it's true
    Ret(Option<ConditionCode>),
    /// Return from subroutine and enable interrupts
    Reti,
    /// Rotate a register left, through the carry flag
    Rl(Value8),
    /// Rotate register `a` left, through the carry flag
    Rla,
    /// Rotate a register left
    Rlc(Value8),
    /// Rotate register `a` left
    Rlca,
    /// Rotate a register right, through the carry flag
    Rr(Value8),
    /// Rotate register `a` right, through the carry flag
    Rra,
    /// Rotate a register right
    Rrc(Value8),
    /// Rotate register `a` right
    Rrca,
    /// Call a function at an address
    ///
    /// This is a faster alternative to `CALL` for addresses that can be packed
    /// into 3 bits. The translation to an address happens at parse time. This
    /// *could* be combined into [Self::Call], but keeping it separate makes
    /// debugging easier.
    Rst(Address),
    /// Subtract a value and the carry flag from `a`
    Sbc(Operand),
    /// Set carry flag
    Scf,
    /// Set a specific bit in a register to 1
    Set(Bit, Value8),
    /// Shift left arithmetically a register
    Sla(Value8),
    /// Shift right arithmetically a register
    Sra(Value8),
    /// Shift right logically a register
    Srl(Value8),
    /// Subtract a value from `a`
    Sub(Operand),
    /// Swap the upper 4 bits of a register with the lower 4
    Swap(Value8),
    /// Enter CPU low power mode
    ///
    /// https://gbdev.io/pandocs/Reducing_Power_Consumption.html
    Stop,
    /// Bitwise XOR between `a` and another value (modifies `a`)
    Xor(Operand),
    /// An invalid instruction from one of the 11 invalid opcodes
    Invalid,
}

impl Instruction {
    /// Construct a `ADD a, *` instruction
    ///
    /// `ADD` has a few more variants than the other math instructions; this
    /// makes it easy to construct the "standard" form.
    pub fn add(operand: Operand) -> Self {
        Self::Add(Add::A(operand))
    }
}

impl Display for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Adc(operand) => write!(f, "ADC {operand}"),
            Self::Add(add) => write!(f, "ADD {add}"),
            Self::And(operand) => write!(f, "AND {operand}"),
            Self::Bit(bit, value) => write!(f, "BIT {bit},{value}"),
            Self::Call {
                address,
                condition: None,
            } => write!(f, "CALL {address}"),
            Self::Call {
                address,
                condition: Some(condition),
            } => write!(f, "CALL {condition},{address}"),
            Self::Ccf => write!(f, "CCF"),
            Self::Cp(operand) => write!(f, "CP {operand}"),
            Self::Cpl => write!(f, "CPL"),
            Self::Daa => write!(f, "DAA"),
            Self::Dec(dec_inc) => write!(f, "DEC {dec_inc}"),
            Self::Di => write!(f, "DI"),
            Self::Ei => write!(f, "EI"),
            Self::Halt => write!(f, "HALT"),
            Self::Inc(dec_inc) => write!(f, "INC {dec_inc}"),
            Self::Jp(jump) => write!(f, "JP {jump}"),
            Self::Jr {
                offset,
                condition: None,
            } => write!(f, "JR {offset}"),
            Self::Jr {
                offset,
                condition: Some(condition),
            } => write!(f, "JR {condition},{offset}"),
            Self::Ld(load) => write!(f, "LD {load}"),
            Self::Ldh(load_high) => write!(f, "LDH {load_high}"),
            Self::Nop => write!(f, "NOP"),
            Self::Or(operand) => write!(f, "OR {operand}"),
            Self::Pop(register) => write!(f, "POP {register}"),
            Self::Push(register) => write!(f, "PUSH {register}"),
            Self::Res(bit, value) => write!(f, "RES {bit},{value}"),
            Self::Ret(None) => write!(f, "RET"),
            Self::Ret(Some(condition)) => write!(f, "RET {condition}"),
            Self::Reti => write!(f, "RETI"),
            Self::Rl(value) => write!(f, "RL {value}"),
            Self::Rla => write!(f, "RLA"),
            Self::Rlc(value) => write!(f, "RLC {value}"),
            Self::Rlca => write!(f, "RLCA"),
            Self::Rr(value) => write!(f, "RR {value}"),
            Self::Rra => write!(f, "RRA"),
            Self::Rrc(value) => write!(f, "RRC {value}"),
            Self::Rrca => write!(f, "RRCA"),
            Self::Rst(address) => write!(f, "RST {address}"),
            Self::Sbc(operand) => write!(f, "SBC {operand}"),
            Self::Scf => write!(f, "SCF"),
            Self::Set(bit, value) => write!(f, "SET {bit},{value}"),
            Self::Sla(value) => write!(f, "SLA {value}"),
            Self::Sra(value) => write!(f, "SRA {value}"),
            Self::Srl(value) => write!(f, "SRL {value}"),
            Self::Sub(operand) => write!(f, "SUB {operand}"),
            Self::Swap(value) => write!(f, "SWAP {value}"),
            Self::Stop => write!(f, "STOP"),
            Self::Xor(operand) => write!(f, "XOR {operand}"),
            Self::Invalid => write!(f, "INVALID"),
        }
    }
}

/// Right-hand side of a math instruction (`r8` or `imm8`)
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Operand {
    /// Value in a register or memory
    V8(Value8),
    /// Constant value
    Const(u8),
}

impl Display for Operand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operand::V8(value) => write!(f, "{value}"),
            Operand::Const(c) => write!(f, "{c}"),
        }
    }
}

impl From<u8> for Operand {
    fn from(value: u8) -> Self {
        Self::Const(value)
    }
}

impl From<Register8> for Operand {
    fn from(register: Register8) -> Self {
        Self::V8(Value8::Register(register))
    }
}

/// Variations of the `ADD` instruction
///
/// Most add variants of `ADD` are handled by [Instruction::Math].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Add {
    /// Add an 8-bit value to `a`
    A(Operand),
    /// Add a 16-bit register value to `hl`
    Hl(Register16),
    /// Add a signed offset to `sp`
    Sp(i8),
}

impl Display for Add {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::A(operand) => write!(f, "a,{operand}"),
            Self::Hl(register) => write!(f, "hl,{register}"),
            Self::Sp(offset) => write!(f, "sp,{offset}"),
        }
    }
}

/// Variations of the `DEC` (decrement) and `INC` (increment) instructions
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DecInc {
    /// Increment an 8-bit value
    V8(Value8),
    /// Increment a 16-bit register
    R16(Register16),
}

impl Display for DecInc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::V8(value) => write!(f, "{value}"),
            Self::R16(register) => write!(f, "{register}"),
        }
    }
}

impl From<Register8> for DecInc {
    fn from(register: Register8) -> Self {
        Self::V8(Value8::Register(register))
    }
}

/// Variations of the `JP` (jump) instruction
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Jump {
    /// Jump to a specific memory address
    Address(Address),
    /// Jump to a specific memory address if the condition is true
    AddressCc(ConditionCode, Address),
    /// Jump to the address in `hl`
    Hl,
}

impl Display for Jump {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Address(address) => write!(f, "{address}"),
            Self::AddressCc(condition, address) => {
                write!(f, "{condition},{address}")
            }
            Self::Hl => write!(f, "hl"),
        }
    }
}

/// Variations of the `LD` (load) instruction
#[derive(Clone, Copy, Debug, PartialEq)]
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
    /// Load a constant into an 8-bit destination
    V8Const { dest: Value8, source: u8 },
    /// Load from one 8-bit value to another
    V8V8 { dest: Value8, source: Value8 },
    /// Load a constant into a 16-bit register
    R16Const { dest: Register16, source: u16 },
    /// Load from register `a` to the byte pointed to by [Register16Memory]
    R16MemA { dest: Register16Memory },
    /// Load from the byte pointed to by [Register16Memory] into register `a`
    AR16Mem { source: Register16Memory },
}

impl Display for Load {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AddressA { dest } => write!(f, "[{dest}],a"),
            Self::AAddress { source } => write!(f, "a,[{source}]"),
            Self::AddressSp { dest } => write!(f, "[{dest}],sp"),
            Self::HlSpOffset { offset } => write!(f, "hl,sp+{offset}"),
            Self::SpHl => write!(f, "sp,hl"),
            Self::V8Const { dest, source } => write!(f, "{dest},{source}"),
            Self::V8V8 { dest, source } => write!(f, "{dest},{source}"),
            Self::R16Const { dest, source } => write!(f, "{dest},{source}"),
            Self::R16MemA { dest } => write!(f, "[{dest}],a"),
            Self::AR16Mem { source } => write!(f, "a,[{source}]"),
        }
    }
}

/// Variations of the `LDH` (load high) instruction
///
/// This is a faster version of `LD` that can only access memory in the range
/// `[0xFF00, 0xFFFF]`. The load address is specified as just the low byte, and
/// the high byte is assumed to be `0xFF`.
#[derive(Clone, Copy, Debug, PartialEq)]
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

impl Display for LoadHigh {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AC => write!(f, "a,[$FF00+c]"),
            Self::AConst(value) => write!(f, "a,[$FF00+${value:2x}]"),
            Self::CA => write!(f, "[$FF00+c],a"),
            Self::ConstA(value) => write!(f, "[$FF00+${value:2x}],a"),
        }
    }
}

/// Source of an 8-bit value
///
/// `r8` on https://gbdev.io/pandocs/CPU_Instruction_Set.html
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value8 {
    /// Value from a register
    Register(Register8),
    /// Byte pointed to by the address in register `hl`
    Hl,
}

impl Display for Value8 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value8::Register(register) => write!(f, "{register}"),
            Value8::Hl => write!(f, "[hl]"),
        }
    }
}

impl From<Register8> for Value8 {
    fn from(register: Register8) -> Self {
        Self::Register(register)
    }
}

/// 8-bit register value (excluding `f`)
///
/// This is *not* equivalent to `r8` on
/// https://gbdev.io/pandocs/CPU_Instruction_Set.html. See [Value8] instead.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Register8 {
    A,
    B,
    C,
    D,
    E,
    H,
    L,
}

impl Display for Register8 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::A => write!(f, "a"),
            Self::B => write!(f, "b"),
            Self::C => write!(f, "c"),
            Self::D => write!(f, "d"),
            Self::E => write!(f, "e"),
            Self::H => write!(f, "h"),
            Self::L => write!(f, "l"),
        }
    }
}

/// Name of a 16-bit register
///
/// `r16` on https://gbdev.io/pandocs/CPU_Instruction_Set.html
#[derive(Clone, Copy, Debug, PartialEq)]
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

impl Display for Register16 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bc => write!(f, "bc"),
            Self::De => write!(f, "de"),
            Self::Hl => write!(f, "hl"),
            Self::Sp => write!(f, "sp"),
        }
    }
}

/// Name of a general purpose 16-bit register for stack operations
///
/// Most instructions use [Register16], but `PUSH`/`POP` use `af` instead of
/// `sp`.
///
/// `r16stk` on https://gbdev.io/pandocs/CPU_Instruction_Set.html
#[derive(Clone, Copy, Debug, PartialEq)]
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

impl Display for Register16Stack {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Af => write!(f, "af"),
            Self::Bc => write!(f, "bc"),
            Self::De => write!(f, "de"),
            Self::Hl => write!(f, "hl"),
        }
    }
}

/// 16-bit register for load operations
///
/// Most instructions use [Register16], but `LD` uses `hli` and `hld` (AKA `hl+`
/// and `hl-`). `r16mem` on https://gbdev.io/pandocs/CPU_Instruction_Set.html
#[derive(Clone, Copy, Debug, PartialEq)]
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

impl Display for Register16Memory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bc => write!(f, "bc"),
            Self::De => write!(f, "de"),
            Self::Hli => write!(f, "hl+"),
            Self::Hld => write!(f, "hl-"),
        }
    }
}

/// Condition for a conditional jump or call
#[derive(Clone, Copy, Debug, PartialEq)]
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

impl Display for ConditionCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Z => write!(f, "Z"),
            Self::Nz => write!(f, "NZ"),
            Self::C => write!(f, "C"),
            Self::Nc => write!(f, "NC"),
        }
    }
}
