mod emu;
mod gfx;
mod rom;

use crate::{
    emu::GameBoy,
    gfx::{Pixel, Screen},
};
use color_eyre::eyre;
use std::{io, path::Path};

const ROM_PATH: &str = "./roms/pokemon_yellow.gb";

fn main() -> eyre::Result<()> {
    let game_boy = GameBoy::load(Path::new(ROM_PATH))?;
    dbg!(game_boy);
    let screen = Screen::new([Pixel::new(255, 0, 0); gfx::IMAGE_SIZE]);
    let mut stdout = io::stdout();
    screen.draw(&mut stdout)?;
    Ok(())
}
