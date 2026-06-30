mod emu;
#[expect(dead_code)] // TODO remove this
mod gfx;
mod memory;
mod rom;

use crate::emu::GameBoy;
use color_eyre::eyre::{self, eyre};
use std::{env, path::PathBuf};

fn main() -> eyre::Result<()> {
    let rom_path =
        PathBuf::from(env::args().nth(1).ok_or(eyre!("Missing ROM path"))?);
    let mut game_boy = GameBoy::load(&rom_path)?;
    game_boy.run()?;
    // let screen = Screen::new([Pixel::new(255, 0, 0); gfx::IMAGE_SIZE]);
    // let mut stdout = io::stdout();
    // screen.draw(&mut stdout)?;
    Ok(())
}
