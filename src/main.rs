mod emu;
mod screen;
mod util;

use crate::{emu::GameBoy, screen::Screen};
use color_eyre::eyre;
use lexopt::Arg;
use std::{env, io, path::PathBuf, str::FromStr};
use tracing::level_filters::LevelFilter;
use tracing_subscriber::{
    filter::Targets, fmt::format::FmtSpan, layer::SubscriberExt,
    util::SubscriberInitExt,
};

fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    initialize_tracing();
    let args = Args::parse()?;

    let mut screen = Screen::test();
    let mut game_boy = GameBoy::boot(&args.rom)?;
    game_boy.run(&mut screen);
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

/// Set up tracing to stderr
fn initialize_tracing() {
    let stderr_subscriber = tracing_subscriber::fmt::layer()
        .with_writer(io::stderr)
        .with_target(true)
        .with_span_events(FmtSpan::NONE);

    let targets = match env::var("RUST_LOG") {
        Ok(env_var) => Targets::from_str(&env_var).unwrap(),
        Err(_) => Targets::new().with_default(LevelFilter::INFO),
    };
    tracing_subscriber::registry()
        .with(targets)
        .with(stderr_subscriber)
        .init();
}
