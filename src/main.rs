mod emu;
mod input;
mod screen;
mod util;

use crate::{
    emu::GameBoy,
    input::{HeadlessInput, Input, TerminalInput},
    screen::{HeadlessScreen, Screen, TerminalScreen},
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
    let screen_width = emu::SCREEN_WIDTH.into();
    let screen_height = emu::SCREEN_HEIGHT.into();
    let (mut input, mut screen): (Box<dyn Input>, Box<dyn Screen>) =
        if args.headless {
            (
                Box::new(HeadlessInput::new(|| false)),
                Box::new(HeadlessScreen::new(screen_width, screen_height)),
            )
        } else {
            (
                Box::new(TerminalInput::new()),
                Box::new(TerminalScreen::new(screen_width, screen_height)?),
            )
        };
    game_boy.run(&mut *input, &mut *screen, args.debug);

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
