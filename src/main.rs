mod emu;
#[expect(dead_code)] // TODO remove this
mod gfx;
mod memory;
mod rom;

use crate::emu::GameBoy;
use color_eyre::eyre::{self, eyre};
use env_logger::Env;
use std::{env, path::PathBuf};

fn main() -> eyre::Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("warn"))
        .init();

    let rom_path =
        PathBuf::from(env::args().nth(1).ok_or(eyre!("Missing ROM path"))?);
    let mut game_boy = GameBoy::boot(&rom_path)?;
    game_boy.run()?;
    Ok(())
}
