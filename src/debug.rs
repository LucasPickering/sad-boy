use crate::emu::{Address, Cycles, GameBoy};
use std::collections::HashSet;
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
    /// Set a breakpoint at the given address
    ///
    /// When the CPU reaches this address (i.e. when `pc == address`), the
    /// debugger will pause.
    pub fn set_breakpoint(&mut self, address: Address) {
        debug!("Setting breakpoint at {address}");
        self.breakpoints.insert(Breakpoint::Address(address));
    }

    /// Is the debugger currently paused?
    pub fn paused(&self) -> bool {
        self.paused
    }

    /// Unpause and set a breakpoint for the given cycle count
    pub fn unpause_until(&mut self, cycle: Cycles) {
        debug!("Unpausing debugger until cycle {cycle}");
        self.paused = false;
        self.breakpoints.insert(Breakpoint::Cycle(cycle));
    }

    /// Toggle pause state
    pub fn toggle_pause(&mut self) {
        self.paused ^= true;
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
            self.paused |= hit;
        };

        // Check each potential breakpoint type. Iterating over them would be
        // a bit more foolproof, but this is O(1) and also makes it easy to
        // remove temporary BPs
        check(Breakpoint::Cycle(emulator.clock().cycles()));
        check(Breakpoint::Address(emulator.cpu().registers().pc()));
    }
}

impl Default for Debugger {
    fn default() -> Self {
        Self {
            paused: true, // Start paused
            breakpoints: HashSet::new(),
        }
    }
}

/// An indicator of when the debugger should pause
///
/// The debugger can have multiple breakpoints registered at a time, and any one
/// of them can trigger a pause. This is used for user-provided breakpoints as
/// well as internal ones (e.g. when stepping by cycle/instruction).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Breakpoint {
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
