//! ROM loading and parsing

use crate::emu::instruction::{
    Add, Address, Bit, ConditionCode, DecInc, Instruction, Jump, Load,
    LoadHigh, Operand, Register8, Register16, Register16Memory,
    Register16Stack, Value8,
};
use color_eyre::eyre::{self, Context};
use std::{
    error::Error,
    fmt::{self, Debug, Display},
    fs,
    ops::{BitAnd, BitOr, Not},
    path::Path,
};
use tracing::{info, trace};
use winnow::{
    ModalResult, Parser,
    binary::{self, Endianness},
    combinator::{cut_err, preceded, trace},
    error::{
        AddContext, ContextError, ErrMode, FromExternalError, ParserError,
        StrContext,
    },
    stream::{Offset, Stream},
    token::one_of,
};

/// A GameBoy ROM (cartridge)
///
/// The ROM has two sections:
/// - [Header](https://gbdev.io/pandocs/The_Cartridge_Header.html)
/// - [Instructions](https://gbdev.io/pandocs/CPU_Instruction_Set.html)
///
/// The header begins at `0x100`; instructions begin at
#[derive(Debug, PartialEq)]
pub struct Rom {
    /// The entire ROM binary data
    data: Vec<u8>,
}

impl Rom {
    /// Load and parse a ROM from a file
    pub fn load(path: &Path) -> eyre::Result<Self> {
        // TODO can we parse the file without loading the whole thing?
        let data = fs::read(path)
            .context(format!("Error reading ROM from {}", path.display()))?;
        info!("Loaded ROM from {}", path.display());
        assert!(data.len() > 0x3FFF, "TODO");
        Ok(Self { data })
    }

    /// Create an empty ROM
    ///
    /// Useful for testing on the [GameBoy] when the ROM doesn't matter.
    #[cfg(test)]
    pub fn empty() -> Rom {
        Self { data: vec![] }
    }

    /// Parse the CPU instruction at the given address
    ///
    /// Return the instruction as well as the number of bytes it consumed. This
    /// is the number of bytes that the PC should advance.
    pub fn get_instruction(
        &self,
        address: Address,
    ) -> Result<(Instruction, usize), RomParseError> {
        // TODO make sure it's in bounds
        let mut input = &self.data[(address.0 as usize)..];
        let start = input.checkpoint(); // TODO this isn't right
        // Don't use Parser::parse() because its error type doesn't print well
        // for binary data
        let (instruction, taken) = parse_instruction
            .with_taken()
            .parse_next(&mut input)
            .map_err(|error| {
                let error = error
                    .into_inner()
                    .expect("Complete parser should not return Incomplete");
                RomParseError::new(&self.data, input.offset_from(&start), error)
            })?;
        let offset = input.offset_from(&start);
        trace!(
            ?instruction,
            bytes = %BytesDisplay(taken),
            %address,
            "Parsed instruction",
        );
        Ok((instruction, offset))
    }

    /// Get the raw ROM bytes
    pub fn bytes(&self) -> &[u8] {
        &self.data
    }
}

/// TODO
#[derive(Debug)]
pub struct RomParseError {
    /// A subslice of the parsing input, with a certain amount of bytes
    /// before/after the error location
    ///
    /// This *could* be an array since it has a fixed max length, but it could
    /// potentially be shorter than `WINDOW_SIZE*2` if the error is at the
    /// beginning/end. A vec is much easier.
    input: Vec<u8>,
    /// Index of the first byte in `input`, relative to the original input
    input_start: usize,
    /// Index of the byte that failed to parse
    offset: usize,
    /// Inner parsing error
    error: ContextError,
}

impl RomParseError {
    /// How many bytes before/after the error to retain
    const WINDOW_SIZE: usize = 16;

    fn new(input: &[u8], offset: usize, error: ContextError) -> Self {
        // Grab a subset of the input
        let start = offset.saturating_sub(Self::WINDOW_SIZE);
        let end = offset.saturating_add(Self::WINDOW_SIZE).min(input.len());
        Self {
            input: input[start..end].to_owned(),
            input_start: start,
            offset,
            error,
        }
    }
}

impl Display for RomParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const BYTES_PER_ROW: usize = 4;
        // Max ROM size is 8MB so we need 6 hex digits to fit all addresses
        const ADDRESS_WIDTH: usize = 6;

        // TODO is this cast safe?
        writeln!(f, "Parse error at byte {}", Address(self.offset as u16))?;
        // Pretty byte rendering
        for (bytes, offset) in self
            .input
            .chunks(BYTES_PER_ROW)
            // For each byte, include its index in the full input
            .zip((self.input_start..).step_by(BYTES_PER_ROW))
        {
            // Pad address with 0s to hit the width
            write!(
                f,
                "{} | {}",
                Address(offset as u16), // TODO is cast safe?
                BytesDisplay(bytes),
            )?;
            writeln!(f)?;

            // If the offending byte is on this line, point it out
            if (offset..(offset + BYTES_PER_ROW)).contains(&self.offset) {
                // Fixed margin padding plus 9 chars per byte within the row
                let padding = ADDRESS_WIDTH + 6 + (self.offset - offset) * 9;
                writeln!(f, "{:>padding$}", "^")?;
            }
        }
        writeln!(f, "{}", self.error)?;
        Ok(())
    }
}

impl Error for RomParseError {}

/// Wrapper to pretty print a byte slice
struct BytesDisplay<'a>(&'a [u8]);

impl Display for BytesDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, byte) in self.0.iter().enumerate() {
            if i > 0 {
                write!(f, " ")?;
            }
            write!(f, "{byte:0>8b}")?;
        }
        Ok(())
    }
}

type ParseError = ErrMode<ContextError>;

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
                match result {
                    Err(e) if ParserError::<&[u8]>::is_backtrack(&e) => { }
                    res => return res,
                }
                input.reset(&start);
            })*

            Err(ParseError::from_input(input))
        }
    };
}

/// Parse the next CPU instruction
#[expect(clippy::precedence)] // TODO fix this
fn parse_instruction(input: &mut &[u8]) -> ModalResult<Instruction> {
    // A giant switch statement for each possible opcode. Some instructions are
    // just a single byte, but some require multiple.
    // https://gbdev.io/pandocs/CPU_Instruction_Set.html
    // NOTE: That doc includes [HL] under r8, with the parameter code 0b110. I
    // broke that out into separate instructions, because the behavior is
    // appreciably different. 0b110 is not a valid param code for r8.
    alt!(
        // ===== BLOCK 0 =====
        0b0000_0000.value(Instruction::Nop).label("nop"),
        //
        (op1(0b0000_0001, Mask::M54), imm16)
            .map(|(dest, source)| {
                Instruction::Ld(Load::R16Const { source, dest })
            })
            .label("ld r16, imm16"),
        op1(0b0000_0010, Mask::M54)
            .map(|dest| Instruction::Ld(Load::R16MemA { dest }))
            .label("ld [r16mem], a"),
        op1(0b0000_1010, Mask::M54)
            .map(|source| Instruction::Ld(Load::AR16Mem { source }))
            .label("ld a, [r16mem]"),
        preceded(0b0000_1000, address)
            .map(|dest| Instruction::Ld(Load::AddressSp { dest }))
            .label("ld [imm16], sp"),
        //
        op1(0b0000_0011, Mask::M54)
            .map(|operand| Instruction::Inc(DecInc::R16(operand)))
            .label("inc r16"),
        op1(0b0000_1011, Mask::M54)
            .map(|operand| Instruction::Dec(DecInc::R16(operand)))
            .label("dec r16"),
        op1(0b0000_1001, Mask::M54)
            .map(|operand| Instruction::Add(Add::Hl(operand)))
            .label("add hl, r16"),
        //
        op1(0b0000_0100, Mask::M543)
            .map(|operand| Instruction::Inc(DecInc::V8(operand)))
            .label("inc r8"),
        op1(0b0000_0101, Mask::M543)
            .map(|operand| Instruction::Dec(DecInc::V8(operand)))
            .label("dec r8"),
        //
        (op1(0b0000_0110, Mask::M543), imm8)
            .map(|(dest, source)| {
                Instruction::Ld(Load::V8Const { dest, source })
            })
            .label("ld r8, imm8"),
        0b0000_0111.value(Instruction::Rlca).label("rlca"),
        0b0000_1111.value(Instruction::Rrca).label("rrca"),
        0b0001_0111.value(Instruction::Rla).label("rla"),
        0b0001_1111.value(Instruction::Rra).label("rra"),
        0b0010_0111.value(Instruction::Daa).label("daa"),
        0b0010_1111.value(Instruction::Cpl).label("cpl"),
        0b0011_0111.value(Instruction::Scf).label("scf"),
        0b0011_1111.value(Instruction::Ccf).label("ccf"),
        //
        preceded(0b0001_1000, imm8)
            .map(|offset| Instruction::Jr {
                // Parse as imm8 so we get its cut_err() call
                offset: offset as i8,
                condition: None
            })
            .label("jr imm8"),
        (op1(0b0010_0000, Mask::M43), imm8)
            .map(|(cond, offset)| {
                // Parse as imm8 so we get its cut_err() call
                Instruction::Jr {
                    offset: offset as i8,
                    condition: Some(cond),
                }
            })
            .label("jr cond, imm8"),
        //
        0b0001_0000.value(Instruction::Stop).label("stop"),
        // ===== BLOCK 1 =====
        // Halt has to come first because it's a subset of the following opcode
        0b0111_0110.value(Instruction::Halt).label("halt"),
        op2(0b0100_0000, Mask::M543, Mask::M210)
            .map(|(dest, source)| Instruction::Ld(Load::V8V8 { dest, source }))
            .label("ld r8, r8"),
        // ===== BLOCK 2 =====
        math_r8(0b1000_0000, Instruction::add).label("add a, r8"),
        math_r8(0b1000_1000, Instruction::Adc).label("adc a, r8"),
        math_r8(0b1001_0000, Instruction::Sub).label("sub a, r8"),
        math_r8(0b1001_1000, Instruction::Sbc).label("sbc a, r8"),
        math_r8(0b1010_0000, Instruction::And).label("and a, r8"),
        math_r8(0b1010_1000, Instruction::Xor).label("xor a, r8"),
        math_r8(0b1011_0000, Instruction::Or).label("or a, r8"),
        math_r8(0b1011_1000, Instruction::Cp).label("cp a, r8"),
        // ===== BLOCK 3 =====
        math_imm8(0b1100_0110, Instruction::add).label("add a, imm8"),
        math_imm8(0b1100_1110, Instruction::Adc).label("adc a, imm8"),
        math_imm8(0b1101_0110, Instruction::Sub).label("sub a, imm8"),
        math_imm8(0b1101_1110, Instruction::Sbc).label("sbc a, imm8"),
        math_imm8(0b1110_0110, Instruction::And).label("and a, imm8"),
        math_imm8(0b1110_1110, Instruction::Xor).label("xor a, imm8"),
        math_imm8(0b1111_0110, Instruction::Or).label("or a, imm8"),
        math_imm8(0b1111_1110, Instruction::Cp).label("cp a, imm8"),
        //
        op1(0b1100_0000, Mask::M43)
            .map(|cond| Instruction::Ret(Some(cond)))
            .label("ret cond"),
        0b1100_1001.value(Instruction::Ret(None)).label("ret"),
        0b1101_1001.value(Instruction::Reti).label("reti"),
        (op1(0b1100_0010, Mask::M43), address)
            .map(|(cond, dest)| {
                Instruction::Jp(Jump::AddressCc(cond, dest))
            })
            .label("jp cond, imm16"),
        preceded(0b1100_0011, address)
            .map(|dest| Instruction::Jp(Jump::Address(dest)))
            .label("jp imm16"),
        0b1110_1001.value(Instruction::Jp(Jump::Hl)).label("jp hl"),
        (op1(0b1100_0100, Mask::M43), address)
            .map(|(cond, address)| {
                Instruction::Call {
                    address,
                    condition: Some(cond),
                }
            })
            .label("call cond, imm16"),
        preceded(0b1100_1101, address)
            .map(|address| Instruction::Call {
                address,
                condition: None
            })
            .label("call imm16"),
        op1::<Address>(0b1100_0111, Mask::M543)
            .map(Instruction::Rst)
            .label("rst tgt3"),
        //
        op1(0b1100_0001, Mask::M54)
            .map(Instruction::Pop)
            .label("pop r16stk"),
        op1(0b1100_0101, Mask::M54)
            .map(Instruction::Push)
            .label("push r16stk"),
        //
        // The byte 0xCB prefixes a set of nested instructions
        preceded(
            0b1100_1011,
            alt!(
                op1(0b0000_0000, Mask::M210)
                    .map(Instruction::Rlc)
                    .label("rlc"),
                op1(0b0000_1000, Mask::M210)
                    .map(Instruction::Rrc)
                    .label("rrc"),
                op1(0b0001_0000, Mask::M210)
                    .map(Instruction::Rl)
                    .label("rl"),
                op1(0b0001_1000, Mask::M210)
                    .map(Instruction::Rr)
                    .label("rr"),
                op1(0b0010_0000, Mask::M210)
                    .map(Instruction::Sla)
                    .label("sla"),
                op1(0b0010_1000, Mask::M210)
                    .map(Instruction::Sra)
                    .label("sra"),
                op1(0b0011_0000, Mask::M210)
                    .map(Instruction::Swap)
                    .label("swap"),
                op1(0b0011_1000, Mask::M210)
                    .map(Instruction::Srl)
                    .label("srl"),
                op2(0b0100_0000, Mask::M543, Mask::M210)
                    .map(|(bit, register)| Instruction::Bit(bit, register))
                    .label("bit b3, r8"),
                op2(0b1000_0000, Mask::M543, Mask::M210)
                    .map(|(bit, register)| Instruction::Res(bit, register))
                    .label("res b3, r8"),
                op2(0b1100_0000, Mask::M543, Mask::M210)
                    .map(|(bit, register)| Instruction::Set(bit, register))
                    .label("set b3, r8"),
            )
        )
        .label("$CB prefix"),
        //
        0b1110_0010
            .value(Instruction::Ldh(LoadHigh::CA))
            .label("ldh [c], a"),
        preceded(0b1110_0000, imm8)
            .map(|offset| Instruction::Ldh(LoadHigh::ConstA(offset)))
            .label("ldh [imm8], a"),
        preceded(0b1110_1010, address)
            .map(|dest| Instruction::Ld(Load::AddressA { dest }))
            .label("ld [imm16], a"),
        0b1111_0010
            .value(Instruction::Ldh(LoadHigh::AC))
            .label("ldh a, [c]"),
        preceded(0b1111_0000, imm8)
            .map(|offset| Instruction::Ldh(LoadHigh::AConst(offset)))
            .label("ldh a, [imm8]"),
        preceded(0b1111_1010, address)
            .map(|source| Instruction::Ld(Load::AAddress { source }))
            .label("ld a, [imm16]"),
        //
        preceded(0b1110_1000, imm8_signed)
            .map(|offset| Instruction::Add(Add::Sp(offset)))
            .label("add sp, imm8"),
        preceded(0b1111_1000, imm8_signed)
            .map(|offset| Instruction::Ld(Load::HlSpOffset { offset }))
            .label("ld hl, sp+imm8"),
        0b1111_1001
            .value(Instruction::Ld(Load::SpHl))
            .label("ld sp, hl"),
        //
        0b1111_0011.value(Instruction::Di).label("di"),
        0b1111_1011.value(Instruction::Ei).label("ei"),
        // Invalid opcodes
        one_of([
            0b1101_0011, // $D3
            0b1101_1011, // $DB
            0b1101_1101, // $DD
            0b1110_0011, // $E3
            0b1110_0100, // $E4
            0b1110_1011, // $EB
            0b1110_1100, // $EC
            0b1110_1101, // $ED
            0b1111_0100, // $F4
            0b1111_1100, // $FC
            0b1111_1101, // $FD
        ])
        .value(Instruction::Invalid),
    )
    .context(StrContext::Label("instruction"))
    .parse_next(input)
}

/// Newtype for a bitmask
///
/// There's a lot of `u8`s floating around in this file, so this helps keep them
/// all straight.
#[derive(Clone, Copy)]
struct Mask(u8);

impl Mask {
    /// Mask for bits 2-0
    const M210: Self = Self(0b0000_0111);
    /// Mask for bits 4-3
    const M43: Self = Self(0b0001_1000);
    /// Mask for bits 5-4
    const M54: Self = Self(0b0011_0000);
    /// Mask for bits 5-3
    const M543: Self = Self(0b0011_1000);
}

impl Debug for Mask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Mask(0b{:0>8b})", self.0)
    }
}

impl Not for Mask {
    type Output = Self;

    fn not(self) -> Self::Output {
        Self(!self.0)
    }
}

impl BitAnd<Mask> for u8 {
    type Output = u8;

    fn bitand(self, rhs: Mask) -> Self::Output {
        self & rhs.0
    }
}

impl BitAnd for Mask {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        Self(self.0 & rhs.0)
    }
}

impl BitOr for Mask {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

trait ParserExt<I, O, E> {
    /// Shortcut for `parser.context(StrContext::Label(label))`
    fn label(self, label: &'static str) -> impl Parser<I, O, E>;
}

impl<I, O, E, T: Parser<I, O, E>> ParserExt<I, O, E> for T
where
    I: Stream,
    E: AddContext<I, StrContext> + ParserError<I>,
{
    fn label(self, label: &'static str) -> impl Parser<I, O, E> {
        self.context(StrContext::Label(label))
    }
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
fn op1<'a, O: BitParameter>(
    opcode: u8,
    mask: Mask,
) -> impl Parser<&'a [u8], O, ParseError> {
    trace("op1", move |input: &mut &'a [u8]| {
        let byte = binary::u8.parse_next(input)?;
        if let Some([param]) = get_bit_params(opcode, [mask], byte) {
            let param = O::from_byte(param)
                .map_err(|error| error.into_parse_error(input))?;
            Ok(param)
        } else {
            Err(ParserError::from_input(input))
        }
    })
}

/// Create a parser for an opcode with two embedded bit parameters
///
/// See [op1] for more info.
fn op2<'a, O1: BitParameter, O2: BitParameter>(
    opcode: u8,
    mask1: Mask,
    mask2: Mask,
) -> impl Parser<&'a [u8], (O1, O2), ParseError> {
    trace("op2", move |input: &mut &'a [u8]| {
        let byte = binary::u8.parse_next(input)?;
        if let Some([param1, param2]) =
            get_bit_params(opcode, [mask1, mask2], byte)
        {
            let param1 = O1::from_byte(param1)
                .map_err(|error| error.into_parse_error(input))?;
            let param2 = O2::from_byte(param2)
                .map_err(|error| error.into_parse_error(input))?;
            Ok((param1, param2))
        } else {
            Err(ParserError::from_input(input))
        }
    })
}

/// Check if the `input` byte matches the static `opcode`
///
/// If it does, extract each bit parameter. Each param value will be shifted
/// down to the least significant bits.
fn get_bit_params<const N: usize>(
    opcode: u8,
    masks: [Mask; N],
    input: u8,
) -> Option<[u8; N]> {
    // Make sure the static opcode has all 0s in the dynamic bits
    let all_masks = masks.into_iter().fold(Mask(0), Mask::bitor);
    debug_assert_eq!(
        opcode & all_masks,
        0,
        "Static opcode must have 0 for all dynamic bytes; \
        opcode=0b{opcode:0>8b}, masks={masks:?}"
    );
    // If the static bits match the opcode
    if input & !all_masks == opcode {
        // Grab each dynamic param via its mask, with its bits shifted down
        // to the right
        Some(masks.map(|mask| (input & mask.0) >> mask.0.trailing_zeros()))
    } else {
        None
    }
}

/// Parse a 2-byte little-endian address from the input
fn address(input: &mut &[u8]) -> ModalResult<Address> {
    imm16.map(Address).parse_next(input)
}

/// Parse one byte as a constant value
fn imm8(input: &mut &[u8]) -> ModalResult<u8> {
    cut_err(binary::u8).parse_next(input)
}

/// Parse one signed byte as a constant value
fn imm8_signed(input: &mut &[u8]) -> ModalResult<i8> {
    imm8.map(|value| value as i8).parse_next(input)
}

/// Parse two bytes little-endian bytes as a constant value
fn imm16(input: &mut &[u8]) -> ModalResult<u16> {
    cut_err(binary::u16(Endianness::Little)).parse_next(input)
}

/// Parse an 8-bit math operation where the operand is the byte value following
/// the opcode
fn math_imm8<'a>(
    opcode: u8,
    operation: fn(Operand) -> Instruction,
) -> impl Parser<&'a [u8], Instruction, ParseError> {
    preceded(opcode, imm8)
        .map(move |operand| operation(Operand::Const(operand)))
}

/// Parse an 8-bit math operation ([Instruction::Math]) where the operand is an
/// 8-bit register encoded in bits 0-2 of the opcode.
fn math_r8<'a>(
    opcode: u8,
    operation: fn(Operand) -> Instruction,
) -> impl Parser<&'a [u8], Instruction, ParseError> {
    op1::<Value8>(opcode, Mask::M210)
        .map(move |operand| operation(Operand::V8(operand)))
}

/// A type that can be parsed from an opcode bit parameter
///
/// Bit parameters are values packed into either 2 or 3 bits of a dynamic
/// opcode. See the first section of
/// https://gbdev.io/pandocs/CPU_Instruction_Set.html
trait BitParameter: Sized {
    /// Map the bit parameter to a semantic value
    ///
    /// The input byte should be shifted down to the bottom (right-most) bits
    /// (which [op1] and [op2] do automatically). Return `Err` for any invalid
    /// value.
    fn from_byte(input: u8) -> Result<Self, BitParameterError>;
}

/// Parse `bit` parameter
impl BitParameter for Bit {
    fn from_byte(input: u8) -> Result<Self, BitParameterError> {
        if input <= 0b111 {
            Ok(Bit(input))
        } else {
            Err(BitParameterError {
                bits: input,
                expected: "0-7",
            })
        }
    }
}

/// Parse `cond` parameter
impl BitParameter for ConditionCode {
    fn from_byte(input: u8) -> Result<Self, BitParameterError> {
        match input {
            0b00 => Ok(Self::Nz),
            0b01 => Ok(Self::Z),
            0b10 => Ok(Self::Nc),
            0b11 => Ok(Self::C),
            _ => Err(BitParameterError {
                bits: input,
                expected: "0-3",
            }),
        }
    }
}

/// Parse `r8` parameter
impl BitParameter for Value8 {
    fn from_byte(input: u8) -> Result<Self, BitParameterError> {
        match input {
            0b000 => Ok(Self::Register(Register8::B)),
            0b001 => Ok(Self::Register(Register8::C)),
            0b010 => Ok(Self::Register(Register8::D)),
            0b011 => Ok(Self::Register(Register8::E)),
            0b100 => Ok(Self::Register(Register8::H)),
            0b101 => Ok(Self::Register(Register8::L)),
            0b110 => Ok(Self::Hl),
            0b111 => Ok(Self::Register(Register8::A)),
            _ => Err(BitParameterError {
                bits: input,
                expected: "0-7",
            }),
        }
    }
}

/// Parse `r16` parameter
impl BitParameter for Register16 {
    fn from_byte(input: u8) -> Result<Self, BitParameterError> {
        match input {
            0b00 => Ok(Self::Bc),
            0b01 => Ok(Self::De),
            0b10 => Ok(Self::Hl),
            0b11 => Ok(Self::Sp),
            _ => Err(BitParameterError {
                bits: input,
                expected: "0-3",
            }),
        }
    }
}

/// Parse `r16mem` parameter
impl BitParameter for Register16Memory {
    fn from_byte(input: u8) -> Result<Self, BitParameterError> {
        match input {
            0b00 => Ok(Self::Bc),
            0b01 => Ok(Self::De),
            0b10 => Ok(Self::Hli),
            0b11 => Ok(Self::Hld),
            _ => Err(BitParameterError {
                bits: input,
                expected: "0-3",
            }),
        }
    }
}

/// Parse `r16stk` parameter
impl BitParameter for Register16Stack {
    fn from_byte(input: u8) -> Result<Self, BitParameterError> {
        match input {
            0b00 => Ok(Self::Bc),
            0b01 => Ok(Self::De),
            0b10 => Ok(Self::Hl),
            0b11 => Ok(Self::Af),
            _ => Err(BitParameterError {
                bits: input,
                expected: "0-3",
            }),
        }
    }
}

/// Parse `tgt3` parameter (for the `RST` instruction)
///
/// It's a 1-byte address encoded into 3 bits of the opcode
impl BitParameter for Address {
    fn from_byte(input: u8) -> Result<Self, BitParameterError> {
        if input <= 0b111 {
            Ok(Address((input * 8).into()))
        } else {
            Err(BitParameterError {
                bits: input,
                expected: "0-7",
            })
        }
    }
}

/// Extracted a bit parameter with an invalid value
///
/// All the bit parameter types use all the possible values for the extracted
/// bits (4 possible values for 2-bit parameters, 8 values for 3-bit params).
/// This error indicates a bug in parsing, probably meaning too many bits were
/// extracted.
#[derive(Debug)]
struct BitParameterError {
    bits: u8,
    expected: &'static str,
}

impl BitParameterError {
    /// Convert to a [ParseError]
    fn into_parse_error(self, input: &mut &[u8]) -> ParseError {
        // If a bit param failed to parse, then it means the instruction opcode
        // matched, so the error is fatal
        ParseError::from_external_error(input, self).cut()
    }
}

impl Display for BitParameterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Invalid bit parameter {:b}; expected {}",
            self.bits, self.expected,
        )
    }
}

impl Error for BitParameterError {}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::property_test;
    use rstest::rstest;

    /// Pass-through bit param parsing for testing
    impl BitParameter for u8 {
        fn from_byte(input: u8) -> Result<Self, BitParameterError> {
            Ok(input)
        }
    }

    /// Test success cases of the `op1` parser
    #[rstest]
    // Masked bits get shifted down to the right
    #[case::middle_bits(0b0100_0101, Mask::M54, 0b01)]
    fn op1_ok(#[case] opcode: u8, #[case] mask: Mask, #[case] expected: u8) {
        let input = &[0b0101_0101];
        let mut parser = op1::<u8>(opcode, mask);
        assert_eq!(parser.parse(input).unwrap(), expected);
    }

    /// Test success cases of the `op2` parser
    #[rstest]
    // Masked bits get shifted down to the right
    #[case::middle_bits(
        0b0100_0000,
        (Mask::M543, Mask::M210),
        (0b0000_0010, 0b0000_0101),
    )]
    fn op2_ok(
        #[case] opcode: u8,
        #[case] masks: (Mask, Mask),
        #[case] expected: (u8, u8),
    ) {
        let input = &[0b0101_0101];
        let mut parser = op2::<u8, u8>(opcode, masks.0, masks.1);
        assert_eq!(parser.parse(input).unwrap(), expected);
    }

    /// Test parsing individual instructions
    #[rstest]
    #[case::inc_r8(&[0b0010_1100], Instruction::Inc(Register8::L.into()))]
    #[case::inc_hl(&[0b0011_0100], Instruction::Inc(DecInc::V8(Value8::Hl)))]
    #[case::inc_r16(
        &[0b0010_0011], Instruction::Inc(DecInc::R16(Register16::Hl))
    )]
    #[case::dec_r8(&[0b0010_1101], Instruction::Dec(Register8::L.into()))]
    #[case::dec_hl(&[0b0011_0101], Instruction::Dec(DecInc::V8(Value8::Hl)))]
    #[case::dec_r16(
        &[0b0010_1011], Instruction::Dec(DecInc::R16(Register16::Hl))
    )]
    #[case::add_hl_r16(
        &[0b0001_1001], Instruction::Add(Add::Hl(Register16::De))
    )]
    #[case::add_sp_imm8(
        &[0b1110_1000, 0b0101_0101], Instruction::Add(Add::Sp(0x55))
    )]
    #[case::add_a_r8(&[0b1000_0001], Instruction::add(Register8::C.into()))]
    #[case::adc_a_r8(&[0b1000_1001], Instruction::Adc(Register8::C.into()))]
    #[case::sub_a_r8(&[0b1001_0001], Instruction::Sub(Register8::C.into()))]
    #[case::sbc_a_r8(&[0b1001_1001], Instruction::Sbc(Register8::C.into()))]
    #[case::and_a_r8(&[0b1010_0001], Instruction::And(Register8::C.into()))]
    #[case::xor_a_r8(&[0b1010_1001], Instruction::Xor(Register8::C.into()))]
    #[case::or_a_r8(&[0b1011_0001], Instruction::Or(Register8::C.into()))]
    #[case::cp_a_r8(&[0b1011_1001], Instruction::Cp(Register8::C.into()))]
    #[case::add_a_imm8(
         &[0b1100_0110, 0b0101_0101], Instruction::add(Operand::Const(0x55))
     )]
    #[case::adc_a_imm8(
         &[0b1100_1110, 0b0101_0101], Instruction::Adc(Operand::Const(0x55))
     )]
    #[case::sub_a_imm8(
         &[0b1101_0110, 0b0101_0101], Instruction::Sub(Operand::Const(0x55))
     )]
    #[case::sbc_a_imm8(
         &[0b1101_1110, 0b0101_0101], Instruction::Sbc(Operand::Const(0x55))
     )]
    #[case::and_a_imm8(
         &[0b1110_0110, 0b0101_0101], Instruction::And(Operand::Const(0x55))
     )]
    #[case::xor_a_imm8(
         &[0b1110_1110, 0b0101_0101], Instruction::Xor(Operand::Const(0x55))
     )]
    #[case::or_a_imm8(
         &[0b1111_0110, 0b0101_0101], Instruction::Or(Operand::Const(0x55))
     )]
    #[case::cp_a_imm8(
         &[0b1111_1110, 0b0101_0101], Instruction::Cp(Operand::Const(0x55))
     )]
    #[case::ret(&[0b1100_1001], Instruction::Ret(None))]
    #[case::ret_cond_nz(
        &[0b1100_0000], Instruction::Ret(Some(ConditionCode::Nz))
    )]
    #[case::ret_cond_z(
        &[0b1100_1000], Instruction::Ret(Some(ConditionCode::Z))
    )]
    #[case::ret_cond_nc(
        &[0b1101_0000], Instruction::Ret(Some(ConditionCode::Nc))
    )]
    #[case::ret_cond_c(
        &[0b1101_1000], Instruction::Ret(Some(ConditionCode::C))
    )]
    #[case::ld_hl_sp_imm8(
        &[0b1111_1000, 0b1010_1010],
        Instruction::Ld(Load::HlSpOffset { offset: -86 })
    )]
    #[case::bit(
        &[0b1100_1011, 0b0111_0100],
        Instruction::Bit(Bit(6), Register8::H.into()),
    )]
    #[case::res(
        &[0b1100_1011, 0b1011_0100],
        Instruction::Res(Bit(6), Register8::H.into()),
    )]
    #[case::set(
        &[0b1100_1011, 0b1111_0100],
        Instruction::Set(Bit(6), Register8::H.into()),
    )]
    fn instruction(#[case] bytes: &[u8], #[case] expected: Instruction) {
        assert_eq!(parse_instruction.parse(bytes).unwrap(), expected);
    }

    /// Property test for parsing instructions:
    /// - Any input of 3 bytes or more parses to *something*
    /// - Parsed instruction is only `Invalid` if it's one of the 11 invalid
    ///   opcodes
    #[property_test]
    fn instruction_prop(bytes: [u8; 3]) {
        const INVALID_CODES: &[u8] = &[
            0b1101_0011, // $D3
            0b1101_1011, // $DB
            0b1101_1101, // $DD
            0b1110_0011, // $E3
            0b1110_0100, // $E4
            0b1110_1011, // $EB
            0b1110_1100, // $EC
            0b1110_1101, // $ED
            0b1111_0100, // $F4
            0b1111_1100, // $FC
            0b1111_1101, // $FD
        ];
        let input = &mut bytes.as_slice();
        let instruction = parse_instruction(input).unwrap();
        // Accept Invalid only if it was a known invalid code
        if instruction == Instruction::Invalid {
            assert!(INVALID_CODES.contains(&bytes[0]));
        }
    }
}
