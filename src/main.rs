mod backend;
mod emu;
mod input;
mod screen;
mod util;

use crate::{
    backend::{Backend, HeadlessBackend, TerminalBackend},
    emu::GameBoy,
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
    game_boy.run(&mut *backend, args.debug);

    Ok(())
}

/// Game Boy emulator
#[derive(Debug, Parser)]
struct Args {
    /// Path to the ROM file to load
    rom: PathBuf,
    /// Expose emulator internals and controls
    #[clap(long)]
    debug: bool,
    /// Run the emulator without a screen
    ///
    /// This is useful for testing the CPU. Tracing will be printed to stderr.
    #[clap(long)]
    headless: bool,
}
