#[cfg(test)]
use crate::backend::Backend;
use crate::emu::{Address, Cycles, GameBoy};
use std::{
    collections::{HashSet, VecDeque},
    fmt::{self, Display},
};
use tracing::debug;

/// Maximum number of snapshots to store in the queue
const MAX_SNAPSHOTS: usize = 10;

/// Debugger enables pausing, stepping, and inspection
///
/// The executor must be started in debug mode ([Executor::debug]) to enable the
/// debugger.
pub struct Debugger {
    /// When paused, the emulator doesn't advance at all
    paused: bool,
    /// Target clock cycle for stepping
    ///
    /// When this is set and `self.paused` is `false`, then the run state will
    /// be [RunState::Stepping]. That means the emulator is ticking, but with
    /// the intention of stopping soon™. See [Self::state].
    ///
    /// This is cleared by [Self::check_breakpoints] once triggered.
    step_until: Option<Cycles>,
    /// Triggers for when the debugger should be paused automatically
    ///
    /// This uses a `HashSet` to prevent duplicate breakpoints and make each
    /// lookup `O(1)`.
    breakpoints: HashSet<Breakpoint>,
    /// Ring buffer of recent snapshots
    ///
    /// The length of this is capped at [MAX_SNAPSHOTS]. Snapshots are only
    /// taken while paused/stepping, for performancey speedy reasons.
    snapshots: VecDeque<Snapshot>,
}

impl Debugger {
    pub fn new() -> Self {
        Self {
            paused: true, // Start paused
            step_until: None,
            breakpoints: HashSet::new(),
            snapshots: VecDeque::with_capacity(MAX_SNAPSHOTS),
        }
    }

    /// Get the current [RunState]
    ///
    /// The ternary state determines whether the emulator should tick and the
    /// debugger should be shown.
    pub fn run_state(&self) -> RunState {
        if self.paused {
            if self.step_until.is_some() {
                RunState::Stepping
            } else {
                RunState::Paused
            }
        } else {
            RunState::Running
        }
    }

    /// Get the queue of saved snapshots
    pub fn snapshots(&self) -> &VecDeque<Snapshot> {
        &self.snapshots
    }

    /// Capture a snapshot for the current emulator state *if* actively
    /// debugging
    pub fn snapshot(&mut self, emulator: &GameBoy) {
        // Only store snapshots while paused/stepping. Way too god damned slow
        // (I assume) for full speed
        if self.run_state().is_debugging()
            // Only snapshot if this isn't in the list already
            && self.get_snapshot_index(emulator).is_none()
        {
            // If at capacity, pop the oldest first to prevent a grow
            if self.snapshots.len() == self.snapshots.capacity() {
                self.snapshots.pop_back();
            }
            self.snapshots.push_front(Snapshot(emulator.clone()));
        }
    }

    /// Restore the snapshot before the given one in the history list
    ///
    /// If the snapshot isn't in the list, or it's the oldest available, do
    /// nothing.
    pub fn previous_snapshot(&mut self, emulator: &mut GameBoy) {
        // Front is the most recent; +1 goes toward the back
        if let Some(previous) = self
            .get_snapshot_index(emulator)
            .and_then(|current| self.snapshots.get(current + 1))
        {
            *emulator = previous.0.clone();
        }
    }

    /// Restore the snapshot after the given one in the history list
    ///
    /// If the snapshot isn't in the list, or it's the most recent available,
    /// do nothing.
    pub fn next_snapshot(&mut self, emulator: &mut GameBoy) {
        // Front is the most recent; -1 goes toward the front
        if let Some(next) = self
            .get_snapshot_index(emulator)
            .and_then(|current| self.snapshots.get(current.checked_sub(1)?))
        {
            *emulator = next.0.clone();
        }
    }

    /// Get the index of a snapshot matching the given emulator state
    ///
    /// This uses the clock cycle count as a unique identifier for each state.
    /// Return `None` if this state hasn't been snapshotted
    fn get_snapshot_index(&self, emulator: &GameBoy) -> Option<usize> {
        self.snapshots
            .iter()
            .position(|snap| snap.matches(emulator))
    }

    /// Get a list of currently set breakpoints
    pub fn breakpoints(&self) -> impl ExactSizeIterator<Item = Breakpoint> {
        self.breakpoints.iter().copied()
    }

    /// Set a breakpoint at the given address
    ///
    /// When the CPU reaches this address (i.e. when `pc == address`), the
    /// debugger will pause.
    pub fn set_breakpoint(&mut self, address: Address) {
        debug!("Setting breakpoint at {address}");
        self.breakpoints.insert(Breakpoint::new(address));
    }

    /// Check all registered breakpoints and pause the debugger if any have
    /// been triggered
    ///
    /// This should be called on each cycle. It the given emulator state to
    /// check breakpoint statuses.
    pub fn check_breakpoints(&mut self, emulator: &GameBoy) {
        // Clear step_until once we hit it
        if self
            .step_until
            .is_some_and(|cy| emulator.clock().cycles() >= cy)
        {
            self.step_until = None;
        }

        let pc = Breakpoint::new(emulator.cpu().registers().pc());
        if self.breakpoints.contains(&pc) {
            debug!(breakpoint = ?pc, "Hit breakpoint");
            self.paused = true;
        }
    }

    /// Toggle pause state
    pub fn toggle_pause(&mut self) {
        self.paused = match self.run_state() {
            RunState::Paused => false,
            RunState::Stepping => {
                // While stepping, toggle should mean "stop stepping"
                self.step_until = None;
                true
            }
            RunState::Running => true,
        };
    }

    /// Step forward one clock cycle
    pub fn step_cycle(&mut self, emulator: &GameBoy) {
        self.step_until(emulator.clock().cycles() + 1);
    }

    /// Step forward to the end of the current frame
    pub fn step_frame(&mut self, emulator: &GameBoy) {
        self.step_until(emulator.clock().next_frame_end());
    }

    /// Step forward to the end of the current CPU isntruction
    pub fn step_instruction(&mut self, emulator: &GameBoy) {
        tracing::debug!("{:?}", emulator.cpu().current_instruction());
        self.step_until(emulator.cpu().current_instruction().end);
    }

    /// Set the `step_until` field, which will step forward **without
    /// unpausing** until the target clock cycle is reached.
    fn step_until(&mut self, target: Cycles) {
        debug!("Stepping debugger until cycle {target}");
        self.step_until = Some(target);
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
        while self.run_state().should_tick() {
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

/// A ternary state denoting if the emulator is running and whether the
/// debugger is visible
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum RunState {
    /// Emulator is fully paused
    Paused,
    /// Emulator is running to a target clock cycle
    ///
    /// Debugger should be shown because the run period is short.
    Stepping,
    /// Emulator is running full bore
    ///
    /// Debugger is effectively disabled.
    Running,
}

impl RunState {
    /// Should the emulator be advanced?
    ///
    /// `true` for [Self::Stepping] and [Self::Running]
    pub fn should_tick(self) -> bool {
        match self {
            Self::Paused => false,
            Self::Stepping { .. } | Self::Running => true,
        }
    }

    /// Should the debugger be visible?
    ///
    /// `true` for [Self::Paused] and [Self::Stepping]
    pub fn is_debugging(self) -> bool {
        match self {
            RunState::Paused | RunState::Stepping => true,
            RunState::Running => false,
        }
    }
}

/// A snapshot of a past emulator state
///
/// Emulation is deterministic, so each snapshot can be identified by its clock
/// cycle.
///
/// Currently, the snapshot does *not* include frame state. The frame is
/// rendered independently from the debugger state, so stepping back to a
/// previous frame will not re-render that frame.
pub struct Snapshot(GameBoy);

impl Snapshot {
    /// Does this snapshot match the given emulator state?
    pub fn matches(&self, other: &GameBoy) -> bool {
        self.id() == other.clock().cycles()
    }

    /// Get the clock cycle identifying this snapshot
    pub fn id(&self) -> Cycles {
        self.0.clock().cycles()
    }
}

/// An indicator that the debugger should pause at a particular memory
/// instruction
///
/// The debugger can have multiple breakpoints registered at a time, and any one
/// of them can trigger a pause.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Breakpoint {
    /// Address to pause at
    ///
    /// This will only trigger if the PC **equals** that address. If the
    /// active instruction overlaps this
    address: Address,
}

impl Breakpoint {
    fn new(address: Address) -> Self {
        Self { address }
    }
}

impl Display for Breakpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.address)
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
        // Instructions come from the bootstrap so we don't need a ROM
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
        // Instructions come from the bootstrap so we don't need a ROM
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
