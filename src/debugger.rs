#[cfg(test)]
use crate::backend::Backend;
use crate::emu::{Address, Cycles, GameBoy};
use std::{
    collections::HashSet,
    fmt::{self, Display},
};
use tracing::debug;

/// Debugger enables pausing, stepping, and inspection
///
/// The executor must be started in debug mode ([Executor::debug]) to enable the
/// debugger.
pub struct Debugger {
    /// When paused, the emulator doesn't advance at all
    paused: bool,
    /// Triggers for when the debugger should be paused automatically
    ///
    /// This uses a `HashSet` to prevent duplicate breakpoints and make each
    /// lookup `O(1)`.
    breakpoints: HashSet<Breakpoint>,
}

impl Debugger {
    pub fn new() -> Self {
        Self {
            paused: true, // Start paused
            breakpoints: HashSet::new(),
        }
    }

    /// Is the debugger currently paused?
    pub fn paused(&self) -> bool {
        self.paused
    }

    /// Get a list of currently set breakpoints
    pub fn breakpoints(&self) -> impl Iterator<Item = Breakpoint> {
        self.breakpoints.iter().copied()
    }

    /// Set a breakpoint at the given address
    ///
    /// When the CPU reaches this address (i.e. when `pc == address`), the
    /// debugger will pause.
    pub fn set_breakpoint(&mut self, address: Address) {
        debug!("Setting breakpoint at {address}");
        self.breakpoints.insert(Breakpoint::Address(address));
    }

    /// Toggle pause state
    pub fn toggle_pause(&mut self) {
        self.paused ^= true;
    }

    /// Step forward one clock cycle
    pub fn step_cycle(&mut self, emulator: &GameBoy) {
        self.unpause_until(emulator.clock().cycles() + 1);
    }

    /// Step forward to the end of the current frame
    pub fn step_frame(&mut self, emulator: &GameBoy) {
        self.unpause_until(emulator.clock().next_frame_end());
    }

    /// Step forward to the end of the current CPU isntruction
    pub fn step_instruction(&mut self, emulator: &GameBoy) {
        tracing::debug!("{:?}", emulator.cpu().current_instruction());
        self.unpause_until(emulator.cpu().current_instruction().end);
    }

    /// Unpause and set a breakpoint for the given cycle count
    fn unpause_until(&mut self, cycle: Cycles) {
        debug!("Unpausing debugger until cycle {cycle}");
        self.paused = false;
        self.breakpoints.insert(Breakpoint::Cycle(cycle));
    }

    /// Check all registered breakpoints and pause the debugger if any have
    /// been triggered
    ///
    /// This uses the given emulator state to check breakpoint statuses.
    pub fn check_breakpoints(&mut self, emulator: &GameBoy) {
        let mut check = |bp: Breakpoint| {
            // Remove temporary breakpoints, leave permanent ones
            let hit = if bp.temporary() {
                self.breakpoints.remove(&bp)
            } else {
                self.breakpoints.contains(&bp)
            };
            if hit {
                debug!(breakpoint = ?bp, "Hit breakpoint");
                self.paused = true;
            }
        };

        // Check each potential breakpoint type. Iterating over them would be
        // a bit more foolproof, but this is O(1) and also makes it easy to
        // remove temporary BPs
        check(Breakpoint::Cycle(emulator.clock().cycles()));
        check(Breakpoint::Address(emulator.cpu().registers().pc()));
    }

    /// Run the emulator until the debugger pauses
    ///
    /// This is a simple main loop for unit tests
    #[cfg(test)]
    fn run_to_pause(
        &mut self,
        emulator: &mut GameBoy,
        backend: &mut dyn Backend,
    ) {
        while !self.paused {
            emulator.tick(backend);
            self.check_breakpoints(emulator);
        }
    }
}

impl Default for Debugger {
    fn default() -> Self {
        Self::new()
    }
}

/// An indicator of when the debugger should pause
///
/// The debugger can have multiple breakpoints registered at a time, and any one
/// of them can trigger a pause. This is used for user-provided breakpoints as
/// well as internal ones (e.g. when stepping by cycle/instruction).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Breakpoint {
    /// Pause when the clock hits a certain cycle count
    Cycle(Cycles),
    /// Pause when the program counter hits a certain address
    Address(Address),
}

impl Breakpoint {
    /// Should this breakpoint be removed the first time it's hit?
    fn temporary(self) -> bool {
        match self {
            Self::Cycle(_) => true,
            Self::Address(_) => false,
        }
    }
}

impl Display for Breakpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cycle(cycles) => write!(f, "Cycle {cycles}"),
            Self::Address(address) => write!(f, "{address}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        backend::HeadlessBackend,
        emu::{
            Clock, GameBoy, InstructionInfo,
            instruction::{Instruction, Load, Register8, Register16},
        },
    };

    /// Test stepping by a single clock cycle
    #[test]
    fn step_cycle() {
        let mut backend = HeadlessBackend::new();
        let mut emulator = GameBoy::test(vec![]);
        let mut debugger = Debugger::new();

        assert_eq!(emulator.clock().cycles(), Cycles(0));

        debugger.step_cycle(&emulator);
        debugger.run_to_pause(&mut emulator, &mut backend);
        assert_eq!(emulator.clock().cycles(), Cycles(1));

        debugger.step_cycle(&emulator);
        debugger.run_to_pause(&mut emulator, &mut backend);
        assert_eq!(emulator.clock().cycles(), Cycles(2));
    }

    /// Test stepping by GPU frame
    #[test]
    fn step_frame() {
        let mut backend = HeadlessBackend::new();
        // Instructions come from the bootloader so we don't need a ROM
        let mut emulator = GameBoy::test(vec![0; 1024]);
        let mut debugger = Debugger::new();

        // Assert initial state
        assert_eq!(emulator.clock().cycles(), Cycles(0));

        // Finish the first frame
        debugger.step_frame(&emulator);
        debugger.run_to_pause(&mut emulator, &mut backend);
        assert_eq!(emulator.clock().cycles(), Clock::CYCLES_PER_FRAME);

        // Another step finishes the second frame
        debugger.step_frame(&emulator);
        debugger.run_to_pause(&mut emulator, &mut backend);
        assert_eq!(emulator.clock().cycles(), Clock::CYCLES_PER_FRAME * 2);

        // If we're partway through a frame, the step finishes it
        debugger.step_cycle(&emulator);
        debugger.run_to_pause(&mut emulator, &mut backend);
        assert_eq!(emulator.clock().cycles(), Clock::CYCLES_PER_FRAME * 2 + 1);
        debugger.step_frame(&emulator);
        debugger.run_to_pause(&mut emulator, &mut backend);
        assert_eq!(emulator.clock().cycles(), Clock::CYCLES_PER_FRAME * 3);
    }

    /// Test stepping by CPU instruction
    #[test]
    fn step_instruction() {
        let mut backend = HeadlessBackend::new();
        // Instructions come from the bootloader so we don't need a ROM
        let mut emulator = GameBoy::test(vec![]);
        let mut debugger = Debugger::new();

        // Assert initial state
        assert_eq!(emulator.clock().cycles(), Cycles(0));
        assert_eq!(
            emulator.cpu().current_instruction(),
            InstructionInfo {
                instruction: Instruction::Ld(Load::R16Const {
                    dest: Register16::Sp,
                    source: 0xfffe,
                }),
                duration: Cycles(3),
                end: Cycles(3),
                size: 3
            }
        );

        // Check the first couple instructions
        debugger.step_instruction(&emulator);
        debugger.run_to_pause(&mut emulator, &mut backend);
        assert_eq!(emulator.clock().cycles(), Cycles(3));
        assert_eq!(
            emulator.cpu().current_instruction(),
            InstructionInfo {
                instruction: Instruction::Xor(Register8::A.into()),
                duration: Cycles(1),
                end: Cycles(4),
                size: 1
            }
        );

        debugger.step_instruction(&emulator);
        debugger.run_to_pause(&mut emulator, &mut backend);
        assert_eq!(emulator.clock().cycles(), Cycles(4));
        assert_eq!(
            emulator.cpu().current_instruction(),
            InstructionInfo {
                instruction: Instruction::Ld(Load::R16Const {
                    dest: Register16::Hl,
                    source: 0x9fff,
                }),
                duration: Cycles(3),
                end: Cycles(7),
                size: 3
            }
        );

        // If the instruction is partially finished, we still go to the end
        debugger.step_cycle(&emulator);
        debugger.run_to_pause(&mut emulator, &mut backend);
        assert_eq!(emulator.clock().cycles(), Cycles(5));
        debugger.step_instruction(&emulator);
        debugger.run_to_pause(&mut emulator, &mut backend);
        assert_eq!(emulator.clock().cycles(), Cycles(7));
    }
}
