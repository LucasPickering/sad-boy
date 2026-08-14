mod backend;
mod emu;
mod exec;
mod input;
mod util;

use crate::{
    backend::{Backend, HeadlessBackend, TerminalBackend},
    emu::{Address, GameBoy},
    exec::{Debugger, Executor},
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
        Executor::debug(&mut *backend, debugger)
    } else {
        Executor::new(&mut *backend)
    };

    game_boy.run(&mut stepper);

    Ok(())
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
