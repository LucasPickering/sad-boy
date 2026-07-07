//! Math-related instruction implementations for [GameBoy]

use crate::{
    emu::{Flags, GameBoy},
    instruction::{Add, DecInc, Operand, Value8},
};

// Masks for the bottom half of 8/16-bit values
const HALF8: u8 = 0xf;
const HALF16: u16 = 0xff;

impl GameBoy {
    /// Execute an `ADD` instruction
    ///
    /// Return the number of consumed CPU cycles
    pub(super) fn add(&mut self, add: Add) -> usize {
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
                let flags = Flags {
                    zero: new == 0,
                    subtract: false,
                    half_carry: (lhs & HALF16) + (rhs & HALF16) > HALF16,
                    carry,
                };
                (flags, 2)
            }
            Add::Sp(rhs) => {
                let lhs = self.registers.sp.0;
                let (new, carry) = lhs.overflowing_add_signed(rhs.into());
                self.registers.sp.0 = new;
                let flags = Flags {
                    zero: new == 0,
                    subtract: false,
                    half_carry: false, // TODO
                    carry,
                };
                (flags, 4)
            }
        };
        self.registers.set_flags(flags);
        cycles
    }

    /// Execute an `ADC` instruction
    ///
    /// This adds the (operand + carry flag) to `a`. The carry flag is 0/1.
    /// Return the number of consumed CPU cycles
    pub(super) fn add_carry(&mut self, rhs: Operand) -> usize {
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

    /// Execute a bitwise instruction like `AND` or `XOR`, mutating `a`
    ///
    /// ## Params
    ///
    /// - `operation`: bitwise operation, taking `a, operand`
    /// - `rhs`: right-hand operand
    /// - `half_carry`: value for the `half_carry` flag
    ///
    /// ## Return
    ///
    /// Return the number of consumed CPU cycles
    pub(super) fn bitwise(
        &mut self,
        operation: fn(u8, u8) -> u8,
        rhs: Operand,
        half_carry: bool,
    ) -> usize {
        let (rhs, cycles) = self.operand(rhs);
        let lhs = self.registers.a;
        self.registers.a = operation(lhs, rhs);
        self.registers.set_flags(Flags {
            zero: self.registers.a == 0,
            subtract: false,
            half_carry,
            carry: false,
        });
        cycles
    }

    /// Execute a `CP` instruction
    ///
    /// This subtracts the operand from `a` and sets the flags accordingly, but
    /// discards the value without modifying `a`. Return the number of consumed
    /// CPU cycles
    pub(super) fn compare(&mut self, rhs: Operand) -> usize {
        let (rhs, cycles) = self.operand(rhs);
        let (_, flags) = sub8(self.registers.a, rhs);
        self.registers.set_flags(flags);
        cycles
    }

    /// Execute a `DEC` or `INC` instruction
    ///
    /// `delta` should be `-1` for `DEC`, `1` for `INC` Return the number of
    /// consumed CPU cycles.
    pub(super) fn dec_inc(&mut self, dec_inc: DecInc, delta: i8) -> usize {
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

    /// Execute an `SBC` instruction
    ///
    /// This subtracts the (operand + carry flag) from `a`. The carry flag is
    /// 0/1. Return the number of consumed CPU cycles
    pub(super) fn subtract_carry(&mut self, rhs: Operand) -> usize {
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

    /// Execute a `SUB` instruction
    ///
    /// Return the number of consumed CPU cycles
    pub(super) fn subtract(&mut self, rhs: Operand) -> usize {
        let (rhs, cycles) = self.operand(rhs);
        let (difference, flags) = sub8(self.registers.a, rhs);
        self.registers.a = difference;
        self.registers.set_flags(flags);
        cycles
    }

    /// Evaluate an 8-bit math operand
    ///
    /// Return `(operand, cycles)`. All math operations take 1 CPU cycle for
    /// 8-bit register operands, 2 cycles for `[HL]` or constants.
    fn operand(&mut self, operand: Operand) -> (u8, usize) {
        match operand {
            Operand::V8(Value8::Register(register)) => {
                (self.register8(register), 1)
            }
            Operand::V8(Value8::Hl) => (self.hl_mem(), 2),
            Operand::Const(value) => (value, 2),
        }
    }
}

/// Add two 8-bit numbers, returning the sum and flags
fn add8(lhs: u8, rhs: u8) -> (u8, Flags) {
    let (sum, carry) = lhs.overflowing_add(rhs);
    let flags = Flags {
        zero: sum == 0,
        subtract: false,
        half_carry: (lhs & HALF8) + (rhs & HALF8) > HALF8,
        carry,
    };
    (sum, flags)
}

/// Subtract two 8-bit numbers, return the difference and flags
fn sub8(lhs: u8, rhs: u8) -> (u8, Flags) {
    let (difference, carry) = lhs.overflowing_sub(rhs);
    let flags = Flags {
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
    use crate::instruction::Instruction;
    use quickcheck_macros::quickcheck;
    use rstest::rstest;

    /// Test addition to register `a` (`ADD A,n8`)
    #[rstest]
    #[case::zero(0x00, 0x00, 0x00, Flags {
        zero: true,
        subtract: false,
        half_carry: false,
        carry: false,
    })]
    #[case::no_carry(0x44, 0x88, 0xCC, Flags {
        zero: false,
        subtract: false,
        half_carry: false,
        carry: false,
    })]
    #[case::half_carry(0x08, 0x88, 0x90, Flags {
        zero: false,
        subtract: false,
        half_carry: true,
        carry: false,
    })]
    #[case::carry(0xFF, 0x10, 0x0F, Flags {
        zero: false,
        subtract: false,
        half_carry: false,
        carry: true,
    })]
    #[case::double_carry(0xFF, 0x01, 0x00, Flags {
        zero: true,
        subtract: false,
        half_carry: true,
        carry: true,
    })]
    #[case::todo(0x50, 0xb0, 0x00, Flags {
        zero: true,
        subtract: false,
        half_carry: false,
        carry: true,
    })]
    fn add_a(
        #[case] lhs: u8,
        #[case] rhs: u8,
        #[case] expected_value: u8,
        #[case] expected_flags: Flags,
    ) {
        let mut game_boy = GameBoy::empty();
        game_boy.registers.a = lhs;
        game_boy.execute(Instruction::Add(Add::A(Operand::Const(rhs))));
        assert_eq!(game_boy.registers.a, expected_value, "sum");
        assert_eq!(game_boy.registers.flags(), expected_flags, "flags");
    }

    /// Property test for [add8]
    /// - Sum is always `lhs+rhs % 256`
    /// - Zero flag is set if sum is 0
    /// - Carry flag is set if `lhs+rhs > 255`
    /// - Half carry flag is set if TODO
    ///
    /// The goal of this is to take a different angle to flag calculation to
    /// give another level of insurance.
    #[quickcheck]
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
            Flags {
                zero: sum_wrap == 0,
                subtract: false,
                half_carry: ((lhs & 0xf) + (rhs & 0xf)) != (sum16 & 0xf),
                carry: sum16 > 0xff,
            },
            "flags"
        );
    }
}
