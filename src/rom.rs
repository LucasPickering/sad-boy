//! Utilities for ROM management

use crate::emu::{
    Add, Address, Bit, ConditionCode, Dec, Inc, Instruction, Jump, Load, Math,
    MathTarget, Register8, Register16, Register16Memory, Register16Stack,
};
use color_eyre::eyre::{self, Context, eyre};
use log::info;
use std::{fs, path::Path};
use winnow::{
    ModalResult, Parser,
    binary::{Endianness, i8, u8, u16},
    combinator::{preceded, repeat},
    error::{ContextError, ErrMode, ParserError},
    token::take,
};

/// A GameBoy ROM (cartridge)
///
/// The ROM has two sections:
/// - [Header](https://gbdev.io/pandocs/The_Cartridge_Header.html)
/// - [Instructions](https://gbdev.io/pandocs/CPU_Instruction_Set.html)
///
/// The header begins at `0x100`; instructions begin at
pub struct Rom {
    /// Metadata from the range `[0x0100, 0x014F]`
    header: RomHeader,
    /// All instructions from the ROM body, loaded and parsed
    instructions: Vec<Instruction>,
}

impl Rom {
    /// Load and parse a ROM from a file
    pub fn load(path: &Path) -> eyre::Result<Self> {
        // TODO can we parse the file without loading the whole thing?
        let data = fs::read(path)
            .context(format!("Error reading ROM from {}", path.display()))?;
        let rom = parse_rom.parse(&data).map_err(|e| eyre!("{e}"))?;
        info!("Loaded ROM from {}", path.display());
        Ok(rom)
    }
}

struct RomHeader {}

type ParseError = ErrMode<ContextError>;

/// Parse all data from the ROM
fn parse_rom(input: &mut &[u8]) -> ModalResult<Rom> {
    let (header, instructions) = (
        take(0x014Fusize).map(|_| RomHeader {}), // Skip the header for now
        repeat(1.., parse_instruction),
    )
        .parse_next(input)?;
    Ok(Rom {
        header,
        instructions,
    })
}

/// A version of winnow's `alt` combinator that takes any number of branches
macro_rules! alt {
    ($($parser:expr),*, $(,)?) => {
        |input: &mut &[u8]| -> ModalResult<Instruction> {
            use winnow::stream::Stream;
            use winnow::Parser;

            let start = input.checkpoint();

            $({
                let result: ModalResult<_> = $parser.parse_next(input);
                // Backtrack errors get tossed, Ok and fatal errors exit
                if !result.as_ref().is_err_and(ParserError::<&[u8]>::is_backtrack) {
                    return result;
                }
                input.reset(&start);
            })*

            todo!("fail for unknown instructions")
        }
    };
}

/// Parse the next CPU instruction
#[expect(clippy::precedence)] // TODO fix this
fn parse_instruction(input: &mut &[u8]) -> ModalResult<Instruction> {
    const MASK_210: u8 = 0b0000_0111;
    const MASK_54: u8 = 0b0011_0000;
    const MASK_543: u8 = 0b0011_1000;

    // A giant switch statement for each possible opcode. Most instructions are
    // just a single byte, but some require multiple.
    // https://gbdev.io/pandocs/CPU_Instruction_Set.html#block-0
    alt!(
        // ===== BLOCK 0 =====
        0b0000_0000.value(Instruction::Nop),
        //
        (op1(0b0000_0001, MASK_54, r16), imm16).map(|(dest, source)| {
            Instruction::Ld(Load::R16Const { source, dest })
        }),
        op1(0b0000_0010, MASK_54, r16mem)
            .map(|dest| Instruction::Ld(Load::R16MemA { dest })),
        op1(0b0000_1010, MASK_54, r16mem)
            .map(|source| Instruction::Ld(Load::AR16Mem { source })),
        preceded(0b0000_1000, address)
            .map(|dest| Instruction::Ld(Load::AddressSp { dest })),
        //
        op1(0b0000_0011, MASK_54, r16)
            .map(|operand| Instruction::Inc(Inc::R16(operand))),
        op1(0b0000_1011, MASK_54, r16)
            .map(|operand| Instruction::Dec(Dec::R16(operand))),
        op1(0b0000_1001, MASK_54, r16)
            .map(|operand| Instruction::Add(Add::Hl(operand))),
        //
        op1(0b0000_0100, MASK_543, r8)
            .map(|operand| Instruction::Inc(Inc::R8(operand))),
        op1(0b0000_0101, MASK_543, r8)
            .map(|operand| Instruction::Dec(Dec::R8(operand))),
        //
        (op1(0b0000_0110, MASK_543, r8), imm8).map(|(dest, source)| {
            Instruction::Ld(Load::R8Const { dest, source })
        }),
        0b0000_0111.value(Instruction::Rlca),
        0b0000_1111.value(Instruction::Rrca),
        0b0001_0111.value(Instruction::Rla),
        0b0001_1111.value(Instruction::Rra),
        0b0010_0111.value(Instruction::Daa),
        0b0010_1111.value(Instruction::Cpl),
        0b0011_0111.value(Instruction::Scf),
        0b0011_1111.value(Instruction::Ccf),
        //
        preceded(0b0001_1000, i8).map(|offset| Instruction::Jr {
            offset,
            condition: None
        }),
        (op1(0b0010_0000, 0b0001_1000, cond), i8).map(|(cond, offset)| {
            Instruction::Jr {
                offset,
                condition: Some(cond),
            }
        }),
        //
        0b0001_0000.value(Instruction::Stop),
        // ===== BLOCK 1 =====
        // Halt has to come first because it's a subset of the following opcode
        0b0111_0110.value(Instruction::Halt),
        op2(0b0100_0000, (0b0011_1000, r8), (0b0000_0111, r8))
            .map(|(dest, source)| Instruction::Ld(Load::R8R8 { dest, source })),
        // ===== BLOCK 2 =====
        math_r8(0b1000_0000, Math::Add),
        math_r8(0b1000_1000, Math::Adc),
        math_r8(0b1001_0000, Math::Sub),
        math_r8(0b1001_1000, Math::Sbc),
        math_r8(0b1010_0000, Math::And),
        math_r8(0b1010_1000, Math::Xor),
        math_r8(0b1011_0000, Math::Or),
        math_r8(0b1011_1000, Math::Cp),
        // ===== BLOCK 3 =====
        math_imm8(0b1000_0110, Math::Add),
        math_imm8(0b1000_1110, Math::Adc),
        math_imm8(0b1001_0110, Math::Sub),
        math_imm8(0b1001_1110, Math::Sbc),
        math_imm8(0b1010_0110, Math::And),
        math_imm8(0b1010_1110, Math::Xor),
        math_imm8(0b1011_0110, Math::Or),
        math_imm8(0b1011_1110, Math::Cp),
        //
        op1(0b1100_0000, 0b0001_1000, cond)
            .map(|cond| Instruction::Ret(Some(cond))),
        0b1100_1001.value(Instruction::Ret(None)),
        0b1101_1001.value(Instruction::Reti),
        (op1(0b1100_0010, 0b0001_1000, cond), address).map(|(cond, dest)| {
            Instruction::Jp(Jump::AddressCc(cond, dest))
        }),
        preceded(0b1100_0011, address)
            .map(|dest| Instruction::Jp(Jump::Address(dest))),
        0b1110_1001.value(Instruction::Jp(Jump::Hl)),
        (op1(0b1100_0100, 0b0001_1000, cond), address).map(
            |(cond, address)| Instruction::Call {
                address,
                condition: Some(cond)
            }
        ),
        preceded(0b1100_1101, address).map(|address| Instruction::Call {
            address,
            condition: None
        }),
        op1(0b1100_0111, MASK_543, tgt3).map(Instruction::Rst),
        //
        op1(0b1100_0001, MASK_54, r16stk).map(Instruction::Pop),
        op1(0b1100_0101, MASK_54, r16stk).map(Instruction::Push),
        //
        // The byte 0xCB prefixes a set of nested instructions
        preceded(
            0b1100_1011,
            alt!(
                op1(0b0000_0000, MASK_210, r8).map(Instruction::Rlc),
                op1(0b0000_0001, MASK_210, r8).map(Instruction::Rrc),
                op1(0b0000_0010, MASK_210, r8).map(Instruction::Rl),
                op1(0b0000_0011, MASK_210, r8).map(Instruction::Rr),
                op1(0b0000_0100, MASK_210, r8).map(Instruction::Sla),
                op1(0b0000_0101, MASK_210, r8).map(Instruction::Sra),
                op1(0b0000_0110, MASK_210, r8).map(Instruction::Swap),
                op1(0b0000_0111, MASK_210, r8).map(Instruction::Srl),
                op2(0b0100_0000, (MASK_543, bit), (MASK_210, r8))
                    .map(|(bit, register)| Instruction::Bit(bit, register)),
                op2(0b1000_0000, (MASK_543, bit), (MASK_210, r8))
                    .map(|(bit, register)| Instruction::Res(bit, register)),
                op2(0b1100_0000, (MASK_543, bit), (MASK_210, r8))
                    .map(|(bit, register)| Instruction::Set(bit, register)),
            )
        ),
        //
        0b1110_0010.value(Instruction::Nop), // todo!("ldh [c]), a"
        0b1110_0000.value(Instruction::Nop), // todo!("ldh [imm8]), a"
        preceded(0b1110_1010, address)
            .map(|dest| Instruction::Ld(Load::AddressA { dest })),
        0b1111_0010.value(Instruction::Nop), // todo!("ldh a), [c]"
        0b1111_0000.value(Instruction::Nop), // todo!("ldh a), [imm8]"
        preceded(0b1111_1010, address)
            .map(|source| Instruction::Ld(Load::AAddress { source })),
        //
        preceded(0b1110_1000, i8).map(Instruction::AddSp),
        0b1111_1000.value(Instruction::Nop), // todo!("ld hl), sp + imm8"
        0b1111_1001.value(Instruction::Nop), // todo!("ld sp), hl"
        //
        0b1111_0011.value(Instruction::Di),
        0b1111_1011.value(Instruction::Ei),
    )
    .parse_next(input)
}

/// Create a parser for an opcode with a single embedded bit parameter
///
/// Some opcodes are not a static bit pattern; they have one or two parameters
/// embedded with in the 8-bit opcode. For example:
///
/// `ld r16, imm16` is encoded as `00xx0001`, where `xx` encodes one of four
/// possible values for `r16`.
///
/// ## Params
///
/// - `opcode` is the static part of the code to look for. Parameterized bytes
///   should be encoded as `0` (the above example would be given as
///   `0b0000_0001`).
/// - `mask` is the bitmask defining which bits form the parameter. In the above
///   example, this would be `0b00110000`.
/// - `map_param` is a fallible mapping function to apply to the bit parameter
///   value. **The value will be shifted down to the least significant bits), so
///   that the same function can be used for all opcodes with the same parameter
///   type, regardless of which bits store the param.
fn op1<'a, O>(
    opcode: u8,
    mask: u8,
    map_param: impl Fn(u8) -> Result<O, ParseError>,
) -> impl Parser<&'a [u8], O, ParseError> {
    move |input: &mut &'a [u8]| {
        let byte = u8.parse_next(input)?;
        if byte & !mask == opcode {
            // TODO explain
            map_param(get_param(byte, mask))
        } else {
            todo!()
        }
    }
}

/// Create a parser for an opcode with two embedded bit parameters
///
/// See [op1] for more info.
fn op2<'a, O1, O2>(
    opcode: u8,
    (mask1, map_param1): (u8, impl Fn(u8) -> Result<O1, ParseError>),
    (mask2, map_param2): (u8, impl Fn(u8) -> Result<O2, ParseError>),
) -> impl Parser<&'a [u8], (O1, O2), ParseError> {
    move |input: &mut &'a [u8]| {
        let byte = u8.parse_next(input)?;
        if byte & !mask1 & !mask2 == opcode {
            let param1 = map_param1(get_param(byte, mask1))?;
            let param2 = map_param2(get_param(byte, mask2))?;
            Ok((param1, param2))
        } else {
            todo!()
        }
    }
}

/// Extract a bit param value from a parsed opcode byte; the param will be
/// shifted down to the rightmost bits
fn get_param(opcode: u8, mask: u8) -> u8 {
    (opcode & mask) >> mask.trailing_zeros()
}

/// Parse a 2-byte little-endian address from the input
fn address(input: &mut &[u8]) -> ModalResult<Address> {
    imm16.map(Address).parse_next(input)
}

/// Parse a condition code from a 2-bit opcode parameter
///
/// The parameter should be shifted down to the bottom two bits (which [op1]
/// does automatically). Any value greater than `0b11` is invalid.
fn cond(input: u8) -> ModalResult<ConditionCode> {
    match input {
        0b00 => Ok(ConditionCode::Nz),
        0b01 => Ok(ConditionCode::Z),
        0b10 => Ok(ConditionCode::Nc),
        0b11 => Ok(ConditionCode::C),
        _ => todo!("error"),
    }
}

/// Parse a bit index from a 3-bit opcode parameter
///
/// The parameter should be shifted down to the bottom three bits (which [op1]
/// does automatically). Any value greater than `0b111` is invalid.
fn bit(input: u8) -> ModalResult<Bit> {
    if input <= 0b111 {
        Ok(Bit(input))
    } else {
        todo!("error")
    }
}

/// Parse one byte as a constant value
fn imm8(input: &mut &[u8]) -> ModalResult<u8> {
    u8.parse_next(input)
}

/// Parse two bytes little-endian bytes as a constant value
fn imm16(input: &mut &[u8]) -> ModalResult<u16> {
    u16(Endianness::Little).parse_next(input)
}

/// Parse an 8-bit register reference from a 3-bit opcode parameter
///
/// The parameter should be shifted down to the bottom three bits (which [op1]
/// does automatically). Any value greater than `0b111` is invalid.
fn r8(input: u8) -> ModalResult<Register8> {
    match input {
        0b000 => Ok(Register8::B),
        0b001 => Ok(Register8::C),
        0b010 => Ok(Register8::D),
        0b011 => Ok(Register8::E),
        0b100 => Ok(Register8::H),
        0b101 => Ok(Register8::L),
        0b110 => Ok(Register8::Hl),
        0b111 => Ok(Register8::A),
        _ => todo!("error"),
    }
}

/// Parse an 8-bit math operation ([Instruction::Math]) where the operand is the
/// byte value following the opcode
fn math_imm8<'a>(
    opcode: u8,
    operation: Math,
) -> impl Parser<&'a [u8], Instruction, ParseError> {
    preceded(opcode, u8).map(move |operand| Instruction::Math {
        operation,
        target: MathTarget::Const(operand),
    })
}

/// Parse an 8-bit math operation ([Instruction::Math]) where the operand is an
/// 8-bit register encoded in bits 0-2 of the opcode.
fn math_r8<'a>(
    opcode: u8,
    operation: Math,
) -> impl Parser<&'a [u8], Instruction, ParseError> {
    op1(opcode, 0b0000_0111, r8).map(move |operand| Instruction::Math {
        operation,
        target: MathTarget::Register(operand),
    })
}

/// Parse a 16-bit register reference from a 2-bit opcode parameter
///
/// The parameter should be shifted down to the bottom two bits (which [op1]
/// does automatically). Any value greater than `0b11` is invalid.
fn r16(input: u8) -> ModalResult<Register16> {
    match input {
        0b00 => Ok(Register16::Bc),
        0b01 => Ok(Register16::De),
        0b10 => Ok(Register16::Hl),
        0b11 => Ok(Register16::Sp),
        _ => todo!("error"),
    }
}

/// Parse a 16-bit register reference from a 2-bit opcode parameter (for
/// `LD` only!!)
///
/// The parameter should be shifted down to the bottom two bits (which [op1]
/// does automatically). Any value greater than `0b11` is invalid.
fn r16mem(input: u8) -> ModalResult<Register16Memory> {
    match input {
        0b00 => Ok(Register16Memory::Bc),
        0b01 => Ok(Register16Memory::De),
        0b10 => Ok(Register16Memory::Hli),
        0b11 => Ok(Register16Memory::Hld),
        _ => todo!("error"),
    }
}

/// Parse a 16-bit register reference from a 2-bit opcode parameter (for
/// push/pop only!!)
///
/// The parameter should be shifted down to the bottom two bits (which [op1]
/// does automatically). Any value greater than `0b11` is invalid.
fn r16stk(input: u8) -> ModalResult<Register16Stack> {
    match input {
        0b00 => Ok(Register16Stack::Bc),
        0b01 => Ok(Register16Stack::De),
        0b10 => Ok(Register16Stack::Hl),
        0b11 => Ok(Register16Stack::Af),
        _ => todo!("error"),
    }
}

/// TODO
fn tgt3(input: u8) -> ModalResult<Address> {
    if input <= 0b111 {
        Ok(Address((input * 8) as u16))
    } else {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    /// Test success cases of the `op1` parser
    #[rstest]
    // Masked bits get shifted down to the right
    #[case::middle_bits(0b0100_0101, 0b0011_0000, 0b0000_0001)]
    fn op1_ok(#[case] opcode: u8, #[case] mask: u8, #[case] expected: u8) {
        let input = &[0b0101_0101];
        let mut parser = op1(opcode, mask, Ok);
        assert_eq!(parser.parse(input).unwrap(), expected);
    }

    /// Test success cases of the `op2` parser
    #[rstest]
    // Masked bits get shifted down to the right
    #[case::middle_bits(
        0b0100_0000,
        (0b0011_1000, 0b0000_0111),
        (0b0000_0010, 0b0000_0101),
    )]
    fn op2_ok(
        #[case] opcode: u8,
        #[case] masks: (u8, u8),
        #[case] expected: (u8, u8),
    ) {
        let input = &[0b0101_0101];
        let mut parser = op2(opcode, (masks.0, Ok), (masks.1, Ok));
        assert_eq!(parser.parse(input).unwrap(), expected);
    }
}
