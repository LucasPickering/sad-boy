mod backend;
mod debugger;
mod emu;
mod util;

use crate::{
    backend::{HeadlessBackend, TerminalBackend},
    debugger::Debugger,
    emu::{Address, GameBoy},
    util::{TracingOutput, initialize_tracing},
};
use clap::Parser;
use color_eyre::eyre::{self, bail};
use std::path::PathBuf;

fn main() -> eyre::Result<()> {
    let args = Args::parse();
    color_eyre::install()?;
    initialize_tracing(if args.headless {
        TracingOutput::Stderr
    } else {
        TracingOutput::File
    });

    let mut game_boy = GameBoy::boot(&args.rom)?;

    // --headless just runs until it crashes with logging
    if args.headless {
        let mut backend = HeadlessBackend::new();
        loop {
            game_boy.tick(&mut backend);
        }
    } else {
        let mut terminal = TerminalBackend::new()?;

        // Set up debugger
        let debugger = if args.debug {
            let mut debugger = Debugger::default();
            for address in args.breakpoint {
                debugger.set_breakpoint(address);
            }
            Some(debugger)
        } else if !args.breakpoint.is_empty() {
            bail!("--breakpoint requires --debug");
        } else {
            None
        };

        terminal.run(&mut game_boy, debugger);
        Ok(())
    }
}

/// Game Boy emulator
#[derive(Debug, Parser)]
struct Args {
    /// Path to the ROM file to load
    rom: PathBuf,
    /// Enable the debugger
    ///
    /// The debugger will display system state and allows pausing/stepping.
    #[clap(long, short)]
    debug: bool,
    /// Add a debugger breakpoint at the given hexadecimal code address
    ///
    /// Requires --debug
    #[clap(long, short)]
    breakpoint: Vec<Address>,
    /// Run the emulator without a screen
    ///
    /// This is useful for testing the CPU. Tracing will be printed to stderr.
    #[clap(long)]
    headless: bool,
}
