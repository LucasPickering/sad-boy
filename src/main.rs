mod emu;
mod gfx;
mod rom;

use crate::{
    gfx::{Pixel, Screen},
    rom::Rom,
};
use color_eyre::eyre::{self, eyre};
use std::{env, io, path::Path};

fn main() -> eyre::Result<()> {
    let rom_path = env::args().nth(1).ok_or(eyre!("Missing ROM path"))?;
    let rom = Rom::load(Path::new(&rom_path))?;
    dbg!(rom);
    let screen = Screen::new([Pixel::new(255, 0, 0); gfx::IMAGE_SIZE]);
    let mut stdout = io::stdout();
    screen.draw(&mut stdout)?;
    Ok(())
}
