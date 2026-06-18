mod gfx;
mod rom;

use crate::{
    gfx::{Pixel, Screen},
    rom::Rom,
};
use std::{io, path::Path};

const ROM_PATH: &str = "./roms/pokemon_yellow.gb";

fn main() {
    let _rom = Rom::load(Path::new(ROM_PATH));
    let screen = Screen::new([Pixel::new(255, 0, 0); gfx::IMAGE_SIZE]);
    let mut stdout = io::stdout();
    screen.draw(&mut stdout).unwrap();
}
