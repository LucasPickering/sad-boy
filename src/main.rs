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
use color_eyre::eyre;
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
            game_boy.tick(&mut backend).unwrap();
        }
    } else {
        let mut terminal = TerminalBackend::new()?;

        // Set up debugger
        let mut debugger = Debugger::default();
        for address in args.breakpoint {
            debugger.set_breakpoint(address);
        }

        terminal.run(&mut game_boy, debugger);
        Ok(())
    }
}

/// Game Boy emulator
#[derive(Debug, Parser)]
struct Args {
    /// Path to the ROM file to load
    rom: PathBuf,
    /// Add a debugger breakpoint at the given hexadecimal code address
    #[clap(long, short)]
    breakpoint: Vec<Address>,
    /// Run the emulator without a screen
    ///
    /// This is useful for testing the CPU. Tracing will be printed to stderr.
    #[clap(long)]
    headless: bool,
}
