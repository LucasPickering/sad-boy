//! Mathy and bitwise instruction implementations for [GameBoy]

use crate::{
    emu::{
        cpu::{BcdFlags, CpuExe, Cycles},
        instruction::{Add, DecInc, Operand, Value8},
    },
    util::Bit,
};

// Masks for the bottom half of 8/16-bit values
const HALF8: u8 = 0xf;
const HALF16: u16 = 0xff;

impl CpuExe<'_, '_> {
    /// Execute an `ADD` instruction
    pub(super) fn add(&mut self, add: Add) -> Cycles {
        let (flags, cycles) = match add {
            Add::A(operand) => {
                let (rhs, cycles) = self.operand(operand);
                let (sum, flags) = add8(self.registers.a, rhs);
                self.registers.a = sum;
                (flags, cycles)
            }
            Add::Hl(register) => {
                let rhs = self.register16(register);
                let lhs = self.registers.hl();
                let (new, carry) = lhs.overflowing_add(rhs);
                *self.registers.hl_mut() = new;
                let flags = BcdFlags {
                    zero: new == 0,
                    subtract: false,
                    half_carry: (lhs & HALF16) + (rhs & HALF16) > HALF16,
                    carry,
                };
                (flags, 2.into())
            }
            Add::Sp(rhs) => {
                let lhs = self.registers.sp.0;
                let (new, carry) = lhs.overflowing_add_signed(rhs.into());
                self.registers.sp.0 = new;
                let flags = BcdFlags {
                    zero: new == 0,
                    subtract: false,
                    half_carry: false, // TODO
                    carry,
                };
                (flags, 4.into())
            }
        };
        self.registers.set_flags(flags);
        cycles
    }

    /// Execute an `ADC` instruction
    ///
    /// This adds the (operand + carry flag) to `a`. The carry flag is 0/1.
    pub(super) fn add_carry(&mut self, rhs: Operand) -> Cycles {
        let (rhs, cycles) = self.operand(rhs);
        let (sum, flags) = add8(
            self.registers.a,
            // Add the carry flag as a 0/1
            rhs.wrapping_add(self.registers.flags().carry.into()),
        );
        self.registers.a = sum;
        self.registers.set_flags(flags);
        cycles
    }

    /// Execute a binary bitwise instruction like `AND` or `XOR`, mutating `a`
    ///
    /// ## Params
    ///
    /// - `operation`: bitwise operation, taking `a, operand`
    /// - `rhs`: right-hand operand
    /// - `half_carry`: value for the `half_carry` flag
    ///
    /// ## Return
    pub(super) fn bit_binary(
        &mut self,
        operation: fn(u8, u8) -> u8,
        rhs: Operand,
        half_carry: bool,
    ) -> Cycles {
        let (rhs, cycles) = self.operand(rhs);
        let lhs = self.registers.a;
        self.registers.a = operation(lhs, rhs);
        self.registers.set_flags(BcdFlags {
            zero: self.registers.a == 0,
            subtract: false,
            half_carry,
            carry: false,
        });
        cycles
    }

    /// Execute a `BIT`
    ///
    /// The value of the bit is stored in the `zero` flag.
    pub(super) fn bit_get(&mut self, bit: Bit, value: Value8) -> Cycles {
        let (value, cycles) = match value {
            Value8::Register(register) => (self.register8(register), 2.into()),
            Value8::Hl => (self.hl_mem(), 3.into()),
        };
        let carry = self.registers.flags().carry;
        self.registers.set_flags(BcdFlags {
            zero: bit.get(value),
            // These two flags are hard-coded
            subtract: false,
            half_carry: true,
            // This flag retains its value
            carry,
        });
        cycles
    }

    /// Execute a `SET` or `RES` instruction
    ///
    /// These instructions for not modify any flags.
    pub(super) fn bit_set(
        &mut self,
        bit: Bit,
        dest: Value8,
        value: bool,
    ) -> Cycles {
        let (dest, cycles) = match dest {
            Value8::Register(register) => {
                (self.register8_mut(register), 2.into())
            }
            Value8::Hl => (self.hl_mem_mut(), 4.into()),
        };
        *dest = bit.set(*dest, value);
        cycles
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
    ///
    /// ## Return
    pub(super) fn bit_unary(
        &mut self,
        operation: fn(u8, bool) -> (u8, bool),
        dest: Value8,
    ) -> Cycles {
        let carry = self.registers.flags().carry;
        let (dest, cycles) = match dest {
            Value8::Register(register) => {
                (self.register8_mut(register), 2.into())
            }
            Value8::Hl => (self.hl_mem_mut(), 4.into()),
        };
        let (new, carry) = operation(*dest, carry);
        *dest = new;
        self.registers.set_flags(BcdFlags {
            zero: new == 0,
            subtract: false,
            half_carry: false,
            carry,
        });
        cycles
    }

    /// Execute a `CP` instruction
    ///
    /// This subtracts the operand from `a` and sets the flags accordingly, but
    /// discards the value without modifying `a`.
    pub(super) fn compare(&mut self, rhs: Operand) -> Cycles {
        let (rhs, cycles) = self.operand(rhs);
        let (_, flags) = sub8(self.registers.a, rhs);
        self.registers.set_flags(flags);
        cycles
    }

    /// Decimal Adjust Accumulator
    ///
    /// Adjust register `a` after an arithmetic instruction on a Binary-Coded
    /// Decimal value.
    ///
    /// https://blog.ollien.com/posts/gb-daa/
    pub(super) fn daa(&mut self) -> Cycles {
        let (a, flags) = daa(self.registers.a, self.registers.flags());
        self.registers.a = a;
        self.registers.set_flags(flags);
        1.into()
    }

    /// Execute a `DEC` or `INC` instruction
    ///
    /// `delta` should be `-1` for `DEC`, `1` for `INC`
    pub(super) fn dec_inc(&mut self, dec_inc: DecInc, delta: i8) -> Cycles {
        // TODO set flags
        match dec_inc {
            DecInc::V8(Value8::Register(register)) => {
                let register = self.register8_mut(register);
                *register = register.wrapping_add_signed(delta);
                1.into()
            }
            DecInc::V8(Value8::Hl) => {
                self.set_hl_mem(self.hl_mem().wrapping_add_signed(delta));
                3.into()
            }
            DecInc::R16(register) => {
                let register = self.register16_mut(register);
                *register = register.wrapping_add_signed(delta.into());
                2.into()
            }
        }
    }

    /// Execute a `SUB` instruction
    pub(super) fn subtract(&mut self, rhs: Operand) -> Cycles {
        let (rhs, cycles) = self.operand(rhs);
        let (difference, flags) = sub8(self.registers.a, rhs);
        self.registers.a = difference;
        self.registers.set_flags(flags);
        cycles
    }

    /// Execute an `SBC` instruction
    ///
    /// This subtracts the (operand + carry flag) from `a`. The carry flag is
    /// 0/1.
    pub(super) fn subtract_carry(&mut self, rhs: Operand) -> Cycles {
        let (rhs, cycles) = self.operand(rhs);
        let (difference, flags) = sub8(
            self.registers.a,
            // Subtract the carry flag as a 0/1
            rhs.wrapping_sub(self.registers.flags().carry.into()),
        );
        self.registers.a = difference;
        self.registers.set_flags(flags);
        cycles
    }

    /// Evaluate an 8-bit math operand
    ///
    /// Return `(operand, cycles)`. All math operations take 1 CPU cycle for
    /// 8-bit register operands, 2 cycles for `[HL]` or constants.
    fn operand(&mut self, operand: Operand) -> (u8, Cycles) {
        match operand {
            Operand::V8(Value8::Register(register)) => {
                (self.register8(register), 1.into())
            }
            Operand::V8(Value8::Hl) => (self.hl_mem(), 2.into()),
            Operand::Const(value) => (value, 2.into()),
        }
    }
}

/// Add two 8-bit numbers, returning the sum and flags
fn add8(lhs: u8, rhs: u8) -> (u8, BcdFlags) {
    let (sum, carry) = lhs.overflowing_add(rhs);
    let flags = BcdFlags {
        zero: sum == 0,
        subtract: false,
        half_carry: (lhs & HALF8) + (rhs & HALF8) > HALF8,
        carry,
    };
    (sum, flags)
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
        half_carry: (lhs & HALF8) < (rhs & HALF8),
        carry,
    };
    (difference, flags)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emu::{
        cpu::Cpu,
        instruction::Instruction,
        memory::{Memory, MemoryBus},
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
        let mut cpu = Cpu::default();
        let mut memory = MemoryBus {
            rom: &Rom::empty(),
            ram: &mut Memory::zero(),
            high_ram: &mut Memory::zero(),
        };
        cpu.registers.a = lhs;
        cpu.execute(&mut memory, Instruction::Add(Add::A(Operand::Const(rhs))));
        assert_eq!(cpu.registers.a, expected_value, "sum");
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
}
