mod emu;
mod screen;
mod util;

use crate::{
    emu::GameBoy,
    screen::{HeadlessScreen, Screen, TerminalScreen},
    util::{TracingOutput, initialize_tracing},
};
use clap::Parser;
use color_eyre::eyre;
use signal_hook::consts::signal;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

fn main() -> eyre::Result<()> {
    let args = Args::parse();
    color_eyre::install()?;
    initialize_tracing(if args.headless {
        TracingOutput::Stderr
    } else {
        TracingOutput::File
    });

    // Start a signal listener for SIGINT and friends.
    // We need to catch signals to allow the screen to clean up before exit.
    let quit = Arc::new(AtomicBool::new(false));
    register_signal_listeners(&quit);
    let mut screen: Box<dyn Screen> = if args.headless {
        Box::new(HeadlessScreen::new(
            emu::SCREEN_WIDTH.into(),
            emu::SCREEN_HEIGHT.into(),
        ))
    } else {
        Box::new(TerminalScreen::new(
            emu::SCREEN_WIDTH.into(),
            emu::SCREEN_HEIGHT.into(),
        )?)
    };
    let stop_on = move || quit.load(Ordering::Relaxed);
    let mut game_boy = GameBoy::boot(&args.rom)?;
    game_boy.run(&mut *screen, stop_on);
    Ok(())
}

/// Game Boy emulator
#[derive(Debug, Parser)]
struct Args {
    /// Path to the ROM file to load
    rom: PathBuf,
    /// Run the emulator without a screen
    ///
    /// This is useful for testing the CPU. Tracing will be printed to stderr.
    #[clap(long)]
    headless: bool,
}

/// Register exit signal listeners
///
/// The flag will be **enabled** when any exit signal is received.
fn register_signal_listeners(flag: &Arc<AtomicBool>) {
    let signals = [
        signal::SIGINT,
        signal::SIGHUP,
        signal::SIGQUIT,
        signal::SIGTERM,
    ];
    for signal in signals {
        signal_hook::flag::register(signal, flag.clone()).unwrap();
    }
}
