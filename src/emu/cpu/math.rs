//! Mathy and bitwise instruction implementations for [GameBoy]

use crate::{
    emu::{
        cpu::{BcdFlags, CpuExe},
        instruction::{Add, DecInc, Operand, Value8},
    },
    util::Bit,
};

impl CpuExe<'_, '_> {
    /// Execute an `ADD` instruction
    pub(super) fn add(&mut self, add: Add) {
        let flags = match add {
            Add::A(operand) => {
                let rhs = self.operand(operand);
                let (sum, flags) = add8(self.registers.a, rhs);
                self.registers.a = sum;
                flags
            }
            Add::Hl(register) => {
                let rhs = self.register16(register);
                let lhs = self.registers.hl();
                let (new, carry) = lhs.overflowing_add(rhs);
                *self.registers.hl_mut() = new;
                BcdFlags {
                    zero: new == 0,
                    subtract: false,
                    half_carry: half_carry16(lhs, rhs, new),
                    carry,
                }
            }
            Add::Sp(rhs) => {
                let lhs = self.registers.sp.0;
                let (new, carry) = lhs.overflowing_add_signed(rhs.into());
                self.registers.sp.0 = new;
                BcdFlags {
                    zero: new == 0,
                    // This is always false, even if the offset is negative
                    // https://rgbds.gbdev.io/docs/v1.0.1/gbz80.7#ADD_SP,e8
                    subtract: false,
                    // Half-carry uses 8-bit rules here (bit 3->4)
                    half_carry: half_carry8(
                        (lhs & 0xff) as u8,
                        // Need to force rhs to positive to make the half-carry
                        // logic work. I didn't think about it too hard, but I
                        // did write a test so it's gotta be right.
                        if rhs < 0 { !rhs } else { rhs } as u8,
                        (new & 0xff) as u8,
                    ),
                    carry,
                }
            }
        };
        self.registers.set_flags(flags);
    }

    /// Execute an `ADC` instruction
    ///
    /// This adds the (operand + carry flag) to `a`. The carry flag is 0/1.
    pub(super) fn add_carry(&mut self, rhs: Operand) {
        let rhs = self.operand(rhs);
        let (sum, flags) = add8(
            self.registers.a,
            // Add the carry flag as a 0/1
            rhs.wrapping_add(self.registers.flags().carry.into()),
        );
        self.registers.a = sum;
        self.registers.set_flags(flags);
    }

    /// Execute a binary bitwise instruction like `AND` or `XOR`, mutating `a`
    ///
    /// ## Params
    ///
    /// - `operation`: bitwise operation, taking `a, operand`
    /// - `rhs`: right-hand operand
    /// - `half_carry`: value for the `half_carry` flag
    pub(super) fn bit_binary(
        &mut self,
        operation: fn(u8, u8) -> u8,
        rhs: Operand,
        half_carry: bool,
    ) {
        let rhs = self.operand(rhs);
        let lhs = self.registers.a;
        self.registers.a = operation(lhs, rhs);
        self.registers.set_flags(BcdFlags {
            zero: self.registers.a == 0,
            subtract: false,
            half_carry,
            carry: false,
        });
    }

    /// Execute a `BIT`
    ///
    /// The value of the bit is stored in the `zero` flag.
    pub(super) fn bit_get(&mut self, bit: Bit, value: Value8) {
        let value = match value {
            Value8::Register(register) => self.register8(register),
            Value8::Hl => self.hl_mem(),
        };
        let carry = self.registers.flags().carry;
        self.registers.set_flags(BcdFlags {
            zero: !bit.get(value),
            // These two flags are hard-coded
            subtract: false,
            half_carry: true,
            // This flag retains its value
            carry,
        });
    }

    /// Execute a `SET` or `RES` instruction
    ///
    /// These instructions do not modify any flags.
    pub(super) fn bit_set(&mut self, bit: Bit, dest: Value8, value: bool) {
        match dest {
            Value8::Register(register) => {
                let dest = self.register8_mut(register);
                *dest = bit.set(*dest, value);
            }
            Value8::Hl => {
                let src = self.hl_mem();
                self.set_hl_mem(bit.set(src, value));
            }
        }
    }

    /// Execute a unary bitwise instruction like `SWAP` or `SRL`
    ///
    /// These instructions modify the `carry` flag. This will also set the
    /// `zero` flag if the output is 0.
    ///
    /// ## Params
    ///
    /// - `operation`: Function taking the current value and `carry` flag,
    ///   returning the new value and new `carry` flag
    /// - `dest`: Value to modify
    pub(super) fn bit_unary(
        &mut self,
        operation: fn(u8, bool) -> (u8, bool),
        dest: Value8,
    ) {
        let carry = self.registers.flags().carry;
        let (new, carry) = match dest {
            Value8::Register(register) => {
                let dest = self.register8_mut(register);
                let (new, carry) = operation(*dest, carry);
                *dest = new;
                (new, carry)
            }
            Value8::Hl => {
                let (new, carry) = operation(self.hl_mem(), carry);
                self.set_hl_mem(new);
                (new, carry)
            }
        };
        self.registers.set_flags(BcdFlags {
            zero: new == 0,
            subtract: false,
            half_carry: false,
            carry,
        });
    }

    /// Execute a `CP` instruction
    ///
    /// This subtracts the operand from `a` and sets the flags accordingly, but
    /// discards the value without modifying `a`.
    pub(super) fn compare(&mut self, rhs: Operand) {
        let rhs = self.operand(rhs);
        let (_, flags) = sub8(self.registers.a, rhs);
        self.registers.set_flags(flags);
    }

    /// Decimal Adjust Accumulator
    ///
    /// Adjust register `a` after an arithmetic instruction on a Binary-Coded
    /// Decimal value.
    ///
    /// https://blog.ollien.com/posts/gb-daa/
    pub(super) fn daa(&mut self) {
        let (a, flags) = daa(self.registers.a, self.registers.flags());
        self.registers.a = a;
        self.registers.set_flags(flags);
    }

    /// Execute a `DEC` or `INC` instruction
    pub(super) fn dec_inc(&mut self, dec_inc: DecInc, subtract: bool) {
        let delta = if subtract { -1 } else { 1 };
        match dec_inc {
            DecInc::V8(dest) => {
                let (lhs, out) = match dest {
                    Value8::Register(register) => {
                        let register = self.register8_mut(register);
                        let lhs = *register;
                        *register = lhs.wrapping_add_signed(delta);
                        (lhs, *register)
                    }
                    Value8::Hl => {
                        let lhs = self.hl_mem();
                        let out = lhs.wrapping_add_signed(delta);
                        self.set_hl_mem(out);
                        (lhs, out)
                    }
                };
                self.registers.set_flags(BcdFlags {
                    zero: out == 0,
                    subtract,
                    // Casting the delta to u8 yields the same bits so the
                    // bit arithmetic is the same
                    half_carry: half_carry8(lhs, delta as u8, out),
                    ..self.registers.flags() // Carry flag is retained
                });
            }
            DecInc::R16(register) => {
                let register = self.register16_mut(register);
                *register = register.wrapping_add_signed(delta.into());
                // Does not affect BCD flags
            }
        }
    }

    /// Execute a `SUB` instruction
    pub(super) fn subtract(&mut self, rhs: Operand) {
        let rhs = self.operand(rhs);
        let (difference, flags) = sub8(self.registers.a, rhs);
        self.registers.a = difference;
        self.registers.set_flags(flags);
    }

    /// Execute an `SBC` instruction
    ///
    /// This subtracts the (operand + carry flag) from `a`. The carry flag is
    /// 0/1.
    pub(super) fn subtract_carry(&mut self, rhs: Operand) {
        let rhs = self.operand(rhs);
        let (difference, flags) = sub8(
            self.registers.a,
            // Subtract the carry flag as a 0/1
            rhs.wrapping_sub(self.registers.flags().carry.into()),
        );
        self.registers.a = difference;
        self.registers.set_flags(flags);
    }

    /// Evaluate an 8-bit math operand
    fn operand(&mut self, operand: Operand) -> u8 {
        match operand {
            Operand::V8(Value8::Register(register)) => self.register8(register),
            Operand::V8(Value8::Hl) => self.hl_mem(),
            Operand::Const(value) => value,
        }
    }
}

/// Add two 8-bit numbers, returning the sum and flags
fn add8(lhs: u8, rhs: u8) -> (u8, BcdFlags) {
    let (sum, carry) = lhs.overflowing_add(rhs);
    let flags = BcdFlags {
        zero: sum == 0,
        subtract: false,
        half_carry: half_carry8(lhs, rhs, sum),
        carry,
    };
    (sum, flags)
}

/// Calculate the half-carry flag for 8-bit arithmetic
fn half_carry8(lhs: u8, rhs: u8, out: u8) -> bool {
    // https://gist.github.com/meganesu/9e228b6b587decc783aa9be34ae27841?permalink_comment_id=5941562#gistcomment-5941562
    Bit(4).get(lhs ^ rhs ^ out)
}

/// Calculate the half-carry flag for 16-bit arithmetic
pub fn half_carry16(lhs: u16, rhs: u16, out: u16) -> bool {
    // Get the 12th bit (correct, it's NOT the 8th bit)
    // https://rgbds.gbdev.io/docs/v1.0.1/gbz80.7#ADD_HL,r16
    (lhs ^ rhs ^ out) & 0x1000 > 0
}

/// Inner implementation for [GameBoy::daa]
///
/// This is separate for testing.
fn daa(a: u8, flags: BcdFlags) -> (u8, BcdFlags) {
    let BcdFlags {
        subtract,
        half_carry,
        mut carry,
        ..
    } = flags;

    // Seriously, just read the blog post. It's a bit confusing.
    // https://blog.ollien.com/posts/gb-daa/
    let a = if subtract {
        let mut offset = 0;
        if half_carry {
            offset |= 0x06;
        }
        if carry {
            offset |= 0x60;
        }
        a.wrapping_sub(offset)
    } else {
        let mut offset = 0;
        if a & 0xF > 0x09 || half_carry {
            offset |= 0x06;
        }
        if a > 0x99 || carry {
            offset |= 0x60;
            carry = true;
        }

        a.wrapping_add(offset)
    };

    (
        a,
        BcdFlags {
            zero: a == 0,
            subtract,
            half_carry: false,
            // Retaining `carry` disagrees with the blog post, but it's what
            // the asm guide says to do
            // https://rgbds.gbdev.io/docs/v1.0.1/gbz80.7#DAA
            carry,
        },
    )
}

/// Subtract two 8-bit numbers, return the difference and flags
fn sub8(lhs: u8, rhs: u8) -> (u8, BcdFlags) {
    let (difference, carry) = lhs.overflowing_sub(rhs);
    let flags = BcdFlags {
        zero: difference == 0,
        subtract: true,
        half_carry: half_carry8(lhs, rhs, difference),
        carry,
    };
    (difference, flags)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emu::{
        cpu::Cpu,
        gpu::Vram,
        instruction::{Instruction, Register16},
        memory::{Address, MemoryBus, RandomAccessMemory},
        rom::Rom,
    };
    use proptest::{prelude::Strategy, property_test};
    use rstest::rstest;

    /// Test addition to register `a` (`ADD A,n8`)
    #[rstest]
    #[case::zero(0x00, 0x00, 0x00, BcdFlags {
        zero: true,
        subtract: false,
        half_carry: false,
        carry: false,
    })]
    #[case::no_carry(0x44, 0x88, 0xCC, BcdFlags {
        zero: false,
        subtract: false,
        half_carry: false,
        carry: false,
    })]
    #[case::half_carry(0x08, 0x88, 0x90, BcdFlags {
        zero: false,
        subtract: false,
        half_carry: true,
        carry: false,
    })]
    #[case::carry(0xFF, 0x10, 0x0F, BcdFlags {
        zero: false,
        subtract: false,
        half_carry: false,
        carry: true,
    })]
    #[case::double_carry(0xFF, 0x01, 0x00, BcdFlags {
        zero: true,
        subtract: false,
        half_carry: true,
        carry: true,
    })]
    #[case::carry_zero(0x50, 0xb0, 0x00, BcdFlags {
        zero: true,
        subtract: false,
        half_carry: false,
        carry: true,
    })]
    fn add_a(
        #[case] lhs: u8,
        #[case] rhs: u8,
        #[case] expected_value: u8,
        #[case] expected_flags: BcdFlags,
    ) {
        let mut cpu = Cpu::new();
        cpu.registers.a = lhs;
        execute(&mut cpu, Instruction::Add(Add::A(Operand::Const(rhs))));
        assert_eq!(cpu.registers.a, expected_value, "sum");
        assert_eq!(cpu.registers.flags(), expected_flags, "flags");
    }

    /// Test addition to register `hl` (`ADD HL,r16`)
    #[rstest]
    #[case::zero(0x0000, 0x0000, 0x0000, BcdFlags {
        zero: true,
        subtract: false,
        half_carry: false,
        carry: false,
    })]
    #[case::no_carry(0x4488, 0x8844, 0xCCCC, BcdFlags {
        zero: false,
        subtract: false,
        half_carry: false,
        carry: false,
    })]
    // Half-carry is on bits 11->12 (not 7->8)
    #[case::half_carry(0x0FFF, 0x0001, 0x1000, BcdFlags {
        zero: false,
        subtract: false,
        half_carry: true,
        carry: false,
    })]
    #[case::carry(0xF000, 0x100F, 0x000F, BcdFlags {
        zero: false,
        subtract: false,
        half_carry: false,
        carry: true,
    })]
    #[case::double_carry(0xFFFF, 0x01, 0x00, BcdFlags {
        zero: true,
        subtract: false,
        half_carry: true,
        carry: true,
    })]
    #[case::carry_zero(0x5000, 0xb000, 0x0000, BcdFlags {
        zero: true,
        subtract: false,
        half_carry: false,
        carry: true,
    })]
    fn add_hl(
        #[case] lhs: u16,
        #[case] rhs: u16,
        #[case] expected_value: u16,
        #[case] expected_flags: BcdFlags,
    ) {
        let mut cpu = Cpu::new();
        *cpu.registers.hl_mut() = lhs;
        *cpu.registers.bc_mut() = rhs;
        execute(&mut cpu, Instruction::Add(Add::Hl(Register16::Bc)));
        assert_eq!(cpu.registers.hl(), expected_value, "sum");
        assert_eq!(cpu.registers.flags(), expected_flags, "flags");
    }

    /// Test addition to register `sp` (`ADD SP,e8`)
    #[rstest]
    #[case::zero(0x0000, 0x00, 0x0000, BcdFlags {
        zero: true,
        subtract: false,
        half_carry: false,
        carry: false,
    })]
    // Half-carry is on bits 3->4
    #[case::half_carry_add(0x011F, 0x01, 0x0120, BcdFlags {
        zero: false,
        subtract: false,
        half_carry: true,
        carry: false,
    })]
    #[case::half_carry_sub(0x0120, -0x01, 0x011F, BcdFlags {
        zero: false,
        subtract: false,
        half_carry: true,
        carry: false,
    })]
    fn add_sp(
        #[case] lhs: u16,
        #[case] rhs: i8,
        #[case] expected_value: u16,
        #[case] expected_flags: BcdFlags,
    ) {
        let mut cpu = Cpu::new();
        cpu.registers.sp = Address(lhs);
        execute(&mut cpu, Instruction::Add(Add::Sp(rhs)));
        assert_eq!(cpu.registers.sp.0, expected_value, "sum");
        assert_eq!(cpu.registers.flags(), expected_flags, "flags");
    }

    /// Test addition to register `a` (`SUB A,n8`)
    #[rstest]
    #[case::zero(0x00, 0x00, 0x00, BcdFlags {
        zero: true,
        subtract: true,
        half_carry: false,
        carry: false,
    })]
    #[case::no_carry(0x88, 0x44, 0x44, BcdFlags {
        zero: false,
        subtract: true,
        half_carry: false,
        carry: false,
    })]
    #[case::half_carry(0x90, 0x08, 0x88, BcdFlags {
        zero: false,
        subtract: true,
        half_carry: true,
        carry: false,
    })]
    #[case::carry(0x0F, 0xFF, 0x10, BcdFlags {
        zero: false,
        subtract: true,
        half_carry: false,
        carry: true,
    })]
    #[case::double_carry(0x01, 0xFF, 0x02, BcdFlags {
        zero: false,
        subtract: true,
        half_carry: true,
        carry: true,
    })]
    fn sub_a(
        #[case] lhs: u8,
        #[case] rhs: u8,
        #[case] expected_value: u8,
        #[case] expected_flags: BcdFlags,
    ) {
        let mut cpu = Cpu::new();
        cpu.registers.a = lhs;
        execute(&mut cpu, Instruction::Sub(rhs.into()));
        assert_eq!(cpu.registers.a, expected_value, "difference");
        assert_eq!(cpu.registers.flags(), expected_flags, "flags");
    }

    /// Property test for [add8]
    /// - Sum is always `lhs+rhs % 256`
    /// - Zero flag is set if sum is 0
    /// - Carry flag is set if `lhs+rhs > 255`
    /// - Half carry flag is set if the add would overflow the bottom nibble
    ///
    /// The goal of this is to take a different angle to flag calculation to
    /// give another level of insurance.
    #[property_test]
    fn add8_prop(lhs: u8, rhs: u8) {
        let (sum, flags) = add8(lhs, rhs);

        // Convert operands to u16 so we can do the add without wrapping
        let lhs: u16 = lhs.into();
        let rhs: u16 = rhs.into();
        let sum16 = lhs + rhs;
        let sum_wrap = (sum16 % 0x100) as u8;

        assert_eq!(sum, sum_wrap, "sum");
        assert_eq!(
            flags,
            BcdFlags {
                zero: sum_wrap == 0,
                subtract: false,
                half_carry: ((lhs & 0xf) + (rhs & 0xf)) != (sum16 & 0xf),
                carry: sum16 > 0xff,
            },
            "flags"
        );
    }

    /// Property test for [daa]
    ///
    /// Start with a valid BCD number in `a`. Apply a random add or subtract,
    /// then run `DAA`. Afterwards, these properties must be true:
    /// - Neither hex digit is ever greater than 9
    /// - Zero flag is set iff the output is 0
    /// - Subtract flag is retained (from the add/sub operation)
    /// - Half Carry flag is unset
    /// - Carry flag is retained, or set if addition overflowed
    #[property_test]
    fn daa_prop(
        #[strategy = bcd()] lhs: u8,
        #[strategy = bcd()] rhs: u8,
        subtract: bool,
    ) {
        let op = if subtract { sub8 } else { add8 };
        let (a, flags) = op(lhs, rhs);
        let carry = flags.carry; // Retain this for later
        let (a_out, flags) = daa(a, flags);
        assert!(a_out & 0xF <= 0x9, "lower digit must be <= 9: {a_out:X}");
        assert!(a_out & 0xF0 <= 0x90, "upper digit must be <= 9: {a_out:X}");
        assert_eq!(
            flags,
            BcdFlags {
                zero: a_out == 0,
                subtract,
                half_carry: false,
                // Carry flag can either be retained or set, can never be reset
                carry: carry || (!subtract && a > 0x99)
            },
            "flags"
        );
    }

    /// Proptest strategy to generate a Binary-Coded Decimal number
    ///
    /// This is any number where both hex digits are <= 9.
    fn bcd() -> impl Strategy<Value = u8> {
        // Generate digits separately
        (0u8..=9, 0u8..=9).prop_map(|(high, low)| (high << 4) | low)
    }

    /// Execute an instruction on the CPU
    fn execute(cpu: &mut Cpu, instruction: Instruction) {
        let mut memory = RandomAccessMemory::default();
        let rom = Rom::empty();
        let mut vram = Vram::default();
        let mut memory = MemoryBus::new(&mut memory, &rom, &mut vram);
        let mut exe = cpu.exe(&mut memory);
        exe.execute(instruction);
    }
}
