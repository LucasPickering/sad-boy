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
                let lhs = self.registers.a;
                let (new, carry) = lhs.overflowing_add(rhs);
                self.registers.a = new;
                let flags = Flags {
                    zero: new == 0,
                    subtract: false,
                    half_carry: (lhs & HALF8) + (rhs & HALF8) > HALF8,
                    carry,
                };
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

    /// Execute a `SUB` instruction
    ///
    /// Return the number of consumed CPU cycles
    pub(super) fn subtract(&mut self, rhs: Operand) -> usize {
        let (rhs, cycles) = self.operand(rhs);
        let lhs = self.registers.a;
        let (new, carry) = lhs.overflowing_sub(rhs);
        self.registers.a = new;
        self.registers.set_flags(Flags {
            zero: new == 0,
            subtract: true,
            half_carry: (lhs & HALF8) < (rhs & HALF8),
            carry,
        });
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
