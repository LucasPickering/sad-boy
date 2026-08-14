use crate::{
    backend::{Backend, FrameBuffer},
    emu::{Address, Clock, Cycles, DebugInfo},
    input::InputEvent,
};
use std::{collections::HashSet, ops::ControlFlow, time::Duration};

/// TODO
pub struct Executor<'bk> {
    /// In debug mode, the debugger manages pausing and breakpoints
    debugger: Option<Debugger>,
    /// Input and output bindings
    backend: &'bk mut dyn Backend,
}

impl<'bk> Executor<'bk> {
    /// Create a new [Stepper] in regular (non-debug) mode
    pub fn new(backend: &'bk mut dyn Backend) -> Self {
        Self {
            backend,
            debugger: None,
        }
    }

    /// Create a new [Stepper] in debug mode
    pub fn debug(backend: &'bk mut dyn Backend, debugger: Debugger) -> Self {
        backend.debug(&debugger); // Draw initial debug info

        Self {
            backend,
            debugger: Some(debugger),
        }
    }

    /// Update debug info **if debug mode is enabled**
    ///
    /// If debug mode is disabled, the given function will never be called.
    pub fn update_debug_info(&mut self, f: impl FnOnce(&mut DebugInfo)) {
        if let Some(debugger) = &mut self.debugger {
            f(&mut debugger.info);
        }
    }

    /// Draw the emulator frame to the screen
    pub fn draw(&mut self, frame: &FrameBuffer) {
        self.backend.draw(frame);
    }

    /// Begin a clock cycle
    ///
    /// TODO more
    pub fn next(&mut self) -> ControlFlow<()> {
        const INPUT_TIMEOUT_PAUSED: Duration = Duration::from_millis(100);

        // Check if we've hit any breakpoints and need to pause
        if let Some(debugger) = &mut self.debugger {
            debugger.check_breakpoints();
        }

        // Draw debug info to the screen
        if let Some(debugger) = &self.debugger {
            self.backend.debug(debugger);
        }

        // When paused, we'll wait until there's input. We can't just block on
        // the input channel though, because there may also be a quit signal. So
        // we'll hop back and forth between checking the quit flag and checking
        // the input, every 100ms. This frequency is slow enough that the idle
        // CPU usage will be minimal, but fast enough to provide low latency for
        // exit signals.
        while self.is_paused() {
            if self.backend.should_quit(self.debug_info()) {
                return ControlFlow::Break(());
            }

            // Don't drain the queue here, because we want to check after each
            // event if it was unpaused.
            if let Some(event) = self.backend.next_event(INPUT_TIMEOUT_PAUSED) {
                self.handle_input(event)?;
            }
        }

        // Debugger isn't paused: drain the input queue
        while let Some(event) = self.backend.next_event(Duration::ZERO) {
            self.handle_input(event)?;
        }

        // Continue on with the emulator tick
        if self.backend.should_quit(self.debug_info()) {
            ControlFlow::Break(()) // Exit the app
        } else {
            ControlFlow::Continue(())
        }
    }

    fn is_paused(&self) -> bool {
        self.debugger.as_ref().is_some_and(|d| d.paused)
    }

    fn debug_info(&self) -> Option<&DebugInfo> {
        self.debugger.as_ref().map(|d| &d.info)
    }

    fn handle_input(&mut self, event: InputEvent) -> ControlFlow<()> {
        match event {
            InputEvent::DebugPauseToggle => {
                if let Some(debugger) = &mut self.debugger {
                    debugger.paused ^= true;
                }
            }
            InputEvent::DebugStepCycle => {
                if let Some(debugger) = &mut self.debugger {
                    debugger.unpause_until(debugger.info.clock_cycles + 1);
                }
            }
            InputEvent::DebugStepFrame => {
                if let Some(debugger) = &mut self.debugger {
                    debugger.unpause_until(Clock::next_frame_end(
                        debugger.info.clock_cycles,
                    ));
                }
            }
            InputEvent::DebugStepInstruction => {
                if let Some(debugger) = &mut self.debugger {
                    debugger
                        .unpause_until(debugger.info.cpu.next_instruction.end);
                }
            }

            InputEvent::Quit => return ControlFlow::Break(()), // Exit
            InputEvent::Button(_) => todo!("TODO track input state"),
        }
        ControlFlow::Continue(())
    }
}

/// TODO
pub struct Debugger {
    /// Summary info about the current system state
    pub info: DebugInfo,
    /// When paused, the emulator doesn't advance at all
    pub paused: bool,
    /// Triggers for when the debugger should be paused automatically
    ///
    /// This uses a `HashSet` to prevent duplicate breakpoints and make each
    /// lookup `O(1)`.
    breakpoints: HashSet<Breakpoint>,
}

impl Debugger {
    /// Set a breakpoint at the given address.
    ///
    /// When the CPU reaches this address (i.e. when `pc == address`), the
    /// debugger will pause.
    pub fn set_breakpoint(&mut self, address: Address) {
        self.breakpoints.insert(Breakpoint::Address(address));
    }

    /// Unpause and set a breakpoint for the given cycle count
    fn unpause_until(&mut self, cycle: Cycles) {
        self.paused = false;
        self.breakpoints.insert(Breakpoint::Cycle(cycle));
    }

    /// Check all registered breakpoints, and pause the debugger if any have
    /// been triggered
    ///
    /// This uses the current debug info to check breakpoint statuses.
    fn check_breakpoints(&mut self) {
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
        check(Breakpoint::Cycle(self.info.clock_cycles));
        check(Breakpoint::Address(self.info.cpu.pc));
    }
}

impl Default for Debugger {
    fn default() -> Self {
        Self {
            info: DebugInfo::default(),
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
