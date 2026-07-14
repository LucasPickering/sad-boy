mod emu;
mod screen;
mod util;

use crate::{emu::GameBoy, screen::Screen};
use clap::Parser;
use color_eyre::eyre;
use std::{env, io, path::PathBuf, str::FromStr};
use tracing::level_filters::LevelFilter;
use tracing_subscriber::{
    filter::Targets, fmt::format::FmtSpan, layer::SubscriberExt,
    util::SubscriberInitExt,
};

/// TODO
#[derive(Parser)]
struct Args {
    /// Path to the ROM file to load
    rom: PathBuf,
    /// TODO remove this
    #[clap(long)]
    draw: bool,
}

fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    initialize_tracing();
    let args = Args::parse();

    // Test the screen
    if args.draw {
        let screen = Screen::test();
        screen.draw(io::stdout())?;
    }

    let mut game_boy = GameBoy::boot(&args.rom)?;
    game_boy.run();
    Ok(())
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
