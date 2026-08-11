mod backend;
mod emu;
mod input;
mod util;

use crate::{
    backend::{Backend, FrameBuffer, HeadlessBackend, TerminalBackend},
    emu::{Address, DebugInfo, GameBoy},
    input::InputEvent,
    util::{TracingOutput, initialize_tracing},
};
use clap::Parser;
use color_eyre::eyre;
use std::{ops::ControlFlow, path::PathBuf, time::Duration};

fn main() -> eyre::Result<()> {
    let args = Args::parse();
    color_eyre::install()?;
    initialize_tracing(if args.headless {
        TracingOutput::Stderr
    } else {
        TracingOutput::File
    });

    let mut game_boy = GameBoy::boot(&args.rom)?;

    // Select hardware based on the input flags
    let mut backend: Box<dyn Backend> = if args.headless {
        Box::new(HeadlessBackend::new(|_| false))
    } else {
        Box::new(TerminalBackend::new()?)
    };

    // Set up debugger
    let mut stepper = if args.debug {
        let mut debugger = Debugger::default();
        for address in args.breakpoint {
            debugger.set_breakpoint(address);
        }
        Stepper::debug(&mut *backend, debugger)
    } else {
        Stepper::new(&mut *backend)
    };

    game_boy.run(&mut stepper);

    Ok(())
}

/// TODO
///
/// TODO rename: Executor?
struct Stepper<'bk> {
    /// TODO
    debugger: Option<Debugger>,
    backend: &'bk mut dyn Backend,
}

impl<'bk> Stepper<'bk> {
    /// Create a new [Stepper] in regular (non-debug) mode
    fn new(backend: &'bk mut dyn Backend) -> Self {
        Self {
            backend,
            debugger: None,
        }
    }

    /// Create a new [Stepper] in debug mode
    fn debug(backend: &'bk mut dyn Backend, debugger: Debugger) -> Self {
        Self {
            backend,
            debugger: Some(debugger),
        }
    }

    /// Update debug info **if debug mode is enabled**
    ///
    /// If debug mode is disabled, the given function will never be called.
    fn update_debug_info(&mut self, f: impl FnOnce(&mut DebugInfo)) {
        if let Some(debugger) = &mut self.debugger {
            f(&mut debugger.info);
        }
    }

    fn draw(&mut self, frame: &FrameBuffer) {
        self.backend.draw(frame);
    }

    /// TODO
    fn next(&mut self) -> ControlFlow<()> {
        // TODO explain
        if let Some(debugger) = &mut self.debugger {
            debugger.wait_for_input(&mut *self.backend)?;
            self.backend.debug(debugger);
        }

        // TODO
        if self
            .backend
            .should_quit(self.debugger.as_ref().map(|d| &d.info))
        {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }
}

/// TODO
struct Debugger {
    /// TODO
    info: DebugInfo,
    /// TODO
    paused: bool,
    /// TODO
    breakpoints: Vec<Address>,
}

impl Debugger {
    /// Set a breakpoint at the given address.
    ///
    /// When the CPU reaches this address (i.e. when `pc == address`), the
    /// debugger will pause.
    fn set_breakpoint(&mut self, address: Address) {
        self.breakpoints.push(address);
    }

    /// If the debugger is paused, wait for the next input that will advance
    /// the debugger
    ///
    /// This will return [ControlFlow::Break] if the app should exit. It
    /// returns [ControlFlow::Continue] when the emulator should progress.
    fn wait_for_input(&mut self, backend: &mut dyn Backend) -> ControlFlow<()> {
        while self.paused {
            // Check the quit flag every 100ms so we don't miss any signals
            if backend.should_quit(Some(&self.info)) {
                return ControlFlow::Break(());
            }

            match backend.next_event(Duration::from_millis(100)) {
                Some(InputEvent::DebugPauseToggle) => self.paused ^= true,
                Some(InputEvent::DebugStepNext) => {
                    return ControlFlow::Continue(());
                }
                Some(InputEvent::Quit) => return ControlFlow::Break(()),
                // Regular button input is ignored while paused
                Some(InputEvent::Button(_)) | None => {}
            }
        }

        // We exited the pause, so progress
        ControlFlow::Continue(())
    }
}

impl Default for Debugger {
    fn default() -> Self {
        Self {
            info: DebugInfo::default(),
            paused: true, // Start paused
            breakpoints: vec![],
        }
    }
}

/// Game Boy emulator
#[derive(Debug, Parser)]
struct Args {
    /// Path to the ROM file to load
    rom: PathBuf,
    /// TODO
    #[clap(long, short)]
    debug: bool,
    /// TODO
    #[clap(long, short)]
    breakpoint: Vec<Address>,
    /// Run the emulator without a screen
    ///
    /// This is useful for testing the CPU. Tracing will be printed to stderr.
    #[clap(long)]
    headless: bool,
}
