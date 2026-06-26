//! Utilities for ROM management

use crate::emu::{
    Address, Instruction, InstructionAdd, InstructionDec, InstructionInc,
    InstructionLd, Register16,
};
use color_eyre::eyre::{self, Context, eyre};
use log::info;
use std::{fs, path::Path};
use winnow::{
    ModalResult, Parser,
    combinator::{preceded, repeat},
    error::ParserError,
    token::{any, take},
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
    const MASK_54: u8 = 0b0011_0000;
    const MASK_543: u8 = 0b0011_1000;

    // A giant switch statement for each possible opcode. Most instructions are
    // just a single byte, but some require multiple.
    // https://gbdev.io/pandocs/CPU_Instruction_Set.html#block-0
    alt!(
        // ===== BLOCK 0 =====
        0b0000_0000.value(Instruction::Nop),
        //
        op1(0b0000_0001, MASK_54, Ok).map(|_dest| todo!("ld r16, imm16")),
        op1(0b0000_0010, MASK_54, Ok).map(|_dest| todo!("ld [r16mem], a")),
        op1(0b0000_1010, MASK_54, Ok).map(|_source| todo!("ld a, [r16mem]")),
        preceded(0b0000_1000, address)
            .map(|dest| Instruction::Ld(InstructionLd::Imm16Sp { dest })),
        //
        op1(0b0000_0011, MASK_54, r16)
            .map(|operand| Instruction::Inc(InstructionInc::R16(operand))),
        op1(0b0000_1011, MASK_54, r16)
            .map(|operand| Instruction::Dec(InstructionDec::R16(operand))),
        op1(0b0000_1001, MASK_54, r16)
            .map(|operand| Instruction::Add(InstructionAdd::Hl(operand))),
        //
        op1(0b0000_0100, MASK_543, Ok).map(|_| todo!("inc r8")),
        op1(0b0000_0101, MASK_543, Ok).map(|_| todo!("dec r8")),
        //
        op1(0b0000_0110, MASK_543, Ok).map(|_| todo!("ld r8, imm8")),
        0b0000_0111.value(Instruction::Rlca),
        0b0000_1111.value(Instruction::Rrca),
        0b0001_0111.value(Instruction::Rla),
        0b0001_1111.value(Instruction::Rra),
        0b0010_0111.value(Instruction::Daa),
        0b0010_1111.value(Instruction::Cpl),
        0b0011_0111.value(Instruction::Scf),
        0b0011_1111.value(Instruction::Ccf),
        //
        0b0001_1000.value(Instruction::Nop), // todo!("jr imm8")
        op1(0b0010_0000, 0b0001_1000, Ok).map(|_| todo!("jr cond, imm8")),
        //
        0b0001_0000.value(Instruction::Stop),
        // ===== BLOCK 1 =====
        // Halt has to come first because it's a subset of the following opcode
        0b0111_0110.value(Instruction::Halt),
        op2(0b0100_0000, (0b0011_1000, Ok), (0b0000_0111, Ok))
            .map(|(_dest, _source)| { todo!("ld r8, r8") }),
        // ===== BLOCK 2 =====
        op1(0b1000_0000, 0b0000_0111, Ok).map(|_operand| todo!("add a, r8")),
        op1(0b1000_1000, 0b0000_0111, Ok).map(|_operand| todo!("adc a, r8")),
        op1(0b1001_0000, 0b0000_0111, Ok).map(|_operand| todo!("sub a, r8")),
        op1(0b1001_1000, 0b0000_0111, Ok).map(|_operand| todo!("sbc a, r8")),
        op1(0b1010_0000, 0b0000_0111, Ok).map(|_operand| todo!("and a, r8")),
        op1(0b1010_1000, 0b0000_0111, Ok).map(|_operand| todo!("xor a, r8")),
        op1(0b1011_0000, 0b0000_0111, Ok).map(|_operand| todo!("or a, r8")),
        op1(0b1011_1000, 0b0000_0111, Ok).map(|_operand| todo!("cp a, r8")),
        // ===== BLOCK 3 =====
        0b1000_0110.value(Instruction::Nop), // todo!("add a, imm8")
        0b1000_1110.value(Instruction::Nop), // todo!("adc a, imm8")
        0b1001_0110.value(Instruction::Nop), // todo!("sub a, imm8")
        0b1001_1110.value(Instruction::Nop), // todo!("sbc a, imm8")
        0b1010_0110.value(Instruction::Nop), // todo!("and a, imm8")
        0b1010_1110.value(Instruction::Nop), // todo!("xor a, imm8")
        0b1011_0110.value(Instruction::Nop), // todo!("or a, imm8")
        0b1011_1110.value(Instruction::Nop), // todo!("cp a, imm8")
        //
        op1(0b1100_0000, 0b0001_1000, Ok).map(|_cond| todo!("ret cond")),
        0b1100_1001.value(Instruction::Ret(None)),
        0b1101_1001.value(Instruction::Reti),
        op1(0b1100_0010, 0b0001_1000, Ok).map(|_con| todo!("jp cond, imm16")),
        0b1100_0011.value(Instruction::Nop), // todo!("jp imm16")
        0b1110_1001.value(Instruction::Nop), // todo!("jp hl")
        op1(0b1100_0100, 0b0001_1000, Ok).map(|_con| todo!("call cond, imm16")),
        0b1100_1101.value(Instruction::Nop), // todo!("call imm16")
        op1(0b1100_0111, MASK_543, Ok).map(|_target| todo!("rst tgt3")),
        //
        op1(0b1100_0001, MASK_54, Ok).map(|_| todo!("pop r16stk")),
        op1(0b1100_0101, MASK_54, Ok).map(|_| todo!("push r16stk")),
        //
        // The byte 0xCB prefixes a set of nested instructions
        preceded(
            0b1100_1011,
            alt!(
                op1(0b0000_0000, 0b0000_0111, Ok).map(|_| todo!("rlc r8")),
                op1(0b0000_0001, 0b0000_0111, Ok).map(|_| todo!("rrc r8")),
                op1(0b0000_0010, 0b0000_0111, Ok).map(|_| todo!("rl r8")),
                op1(0b0000_0011, 0b0000_0111, Ok).map(|_| todo!("rr r8")),
                op1(0b0000_0100, 0b0000_0111, Ok).map(|_| todo!("sla r8")),
                op1(0b0000_0101, 0b0000_0111, Ok).map(|_| todo!("sra r8")),
                op1(0b0000_0110, 0b0000_0111, Ok).map(|_| todo!("swap r8")),
                op1(0b0000_0111, 0b0000_0111, Ok).map(|_| todo!("srl r8")),
                op2(0b0100_0000, (0b0011_1000, Ok), (0b0000_0111, Ok))
                    .map(|(_, _)| { todo!("bit b3, r8") }),
                op2(0b1000_0000, (0b0011_1000, Ok), (0b0000_0111, Ok))
                    .map(|(_, _)| { todo!("res b3, r8") }),
                op2(0b1100_0000, (0b0011_1000, Ok), (0b0000_0111, Ok))
                    .map(|(_, _)| { todo!("set b3, r8") }),
            )
        ),
        //
        0b1110_0010.value(Instruction::Nop), // todo!("ldh [c]), a"
        0b1110_0000.value(Instruction::Nop), // todo!("ldh [imm8]), a"
        0b1110_1010.value(Instruction::Nop), // todo!("ld [imm16]), a"
        0b1111_0010.value(Instruction::Nop), // todo!("ldh a), [c]"
        0b1111_0000.value(Instruction::Nop), // todo!("ldh a), [imm8]"
        0b1111_1010.value(Instruction::Nop), // todo!("ld a), [imm16]"
        //
        0b1110_1000.value(Instruction::Nop), // todo!("add sp), imm8"
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
fn op1<'a, O, E: ParserError<&'a [u8]>>(
    opcode: u8,
    mask: u8,
    map_param: impl Fn(u8) -> Result<O, E>,
) -> impl Parser<&'a [u8], O, E> {
    move |input: &mut &'a [u8]| {
        let byte = any.parse_next(input)?;
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
fn op2<'a, O1, O2, E: ParserError<&'a [u8]>>(
    opcode: u8,
    (mask1, map_param1): (u8, impl Fn(u8) -> Result<O1, E>),
    (mask2, map_param2): (u8, impl Fn(u8) -> Result<O2, E>),
) -> impl Parser<&'a [u8], (O1, O2), E> {
    move |input: &mut &'a [u8]| {
        let byte = any.parse_next(input)?;
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
    let (b0, b1) = (any, any).parse_next(input)?;
    Ok(Address(u16::from_le_bytes([b0, b1])))
}

/// Parse a 16-bit register reference from a 2-bit opcode parameter
///
/// The parameter should be shifted down to the bottom two bits (which [op] does
/// automatically). Any value greater than `0b11` is invalid.
fn r16(input: u8) -> ModalResult<Register16> {
    match input {
        0b00 => Ok(Register16::Bc),
        0b01 => Ok(Register16::De),
        0b10 => Ok(Register16::Hl),
        0b11 => Ok(Register16::Sp),
        _ => todo!("error"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use winnow::error::ContextError;

    /// Test success cases of the `op1` parser
    #[rstest]
    // Masked bits get shifted down to the right
    #[case::middle_bits(0b0100_0101, 0b0011_0000, 0b0000_0001)]
    fn op1_ok(#[case] opcode: u8, #[case] mask: u8, #[case] expected: u8) {
        let input = &[0b0101_0101];
        let mut parser = op1::<u8, ContextError>(opcode, mask, Ok);
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
        let mut parser =
            op2::<u8, u8, ContextError>(opcode, (masks.0, Ok), (masks.1, Ok));
        assert_eq!(parser.parse(input).unwrap(), expected);
    }
}
