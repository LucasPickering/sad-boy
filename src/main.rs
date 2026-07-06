mod emu;
#[expect(dead_code)] // TODO remove this
mod gfx;
mod instruction;
mod memory;
mod rom;

use crate::emu::GameBoy;
use color_eyre::eyre::{self, eyre};
use std::{env, io, path::PathBuf, str::FromStr};
use tracing::level_filters::LevelFilter;
use tracing_subscriber::{
    filter::Targets, fmt::format::FmtSpan, layer::SubscriberExt,
    util::SubscriberInitExt,
};

fn main() -> eyre::Result<()> {
    initialize_tracing();

    let rom_path =
        PathBuf::from(env::args().nth(1).ok_or(eyre!("Missing ROM path"))?);
    let mut game_boy = GameBoy::boot(&rom_path)?;
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
