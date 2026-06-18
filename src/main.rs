mod emu;
mod gfx;
mod rom;

use crate::{
    emu::GameBoy,
    gfx::{Pixel, Screen},
};
use std::{io, path::Path};

const ROM_PATH: &str = "./roms/pokemon_yellow.gb";

fn main() {
    let mut game_boy = GameBoy::new();
    game_boy.load_rom(Path::new(ROM_PATH)).unwrap();
    let screen = Screen::new([Pixel::new(255, 0, 0); gfx::IMAGE_SIZE]);
    let mut stdout = io::stdout();
    screen.draw(&mut stdout).unwrap();
}
