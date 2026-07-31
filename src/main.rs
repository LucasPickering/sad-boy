mod emu;
mod screen;
mod util;

use crate::{
    emu::{Clock, GameBoy},
    screen::TerminalScreen,
    util::initialize_tracing,
};
use color_eyre::eyre;
use lexopt::Arg;
use signal_hook::consts::signal;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    initialize_tracing();
    let args = Args::parse()?;

    // Start a signal listener for SIGINT and friends.
    // We need to catch signals to allow the screen to clean up before exit.
    let quit = Arc::new(AtomicBool::new(false));
    register_signal_listeners(&quit);
    let mut screen = TerminalScreen::new(
        emu::SCREEN_WIDTH.into(),
        emu::SCREEN_HEIGHT.into(),
    )?;
    let stop_on = move |_: &Clock| quit.load(Ordering::Relaxed);
    let mut game_boy = GameBoy::boot(&args.rom)?;
    game_boy.run(&mut screen, stop_on);
    Ok(())
}

/// CLI args
struct Args {
    /// Path to the ROM file to load
    rom: PathBuf,
}

impl Args {
    fn parse() -> Result<Self, lexopt::Error> {
        // lexopt is a little clunk but it's much lighter than clap
        let mut rom: Option<PathBuf> = None;
        let mut parser = lexopt::Parser::from_env();
        while let Some(arg) = parser.next()? {
            match arg {
                Arg::Value(value) if rom.is_none() => {
                    rom = Some(PathBuf::from(value));
                }
                Arg::Long("help") | Arg::Short('n') => {
                    println!("Usage: sad-boy ROM");
                    std::process::exit(0);
                }
                _ => return Err(arg.unexpected()),
            }
        }

        Ok(Self {
            rom: rom.ok_or("missing argument ROM")?,
        })
    }
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
