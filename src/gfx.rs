//! Graphics bindings for the terminal
//!
//! This uses the [Kitty Terminal Graphics Protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/).

use base64::{engine::general_purpose::STANDARD, write::EncoderWriter};
use std::{
    io::{self, Write},
    mem,
};

// Screen width/height
const WIDTH: u16 = 160;
const HEIGHT: u16 = 144;
/// Total number of pixels in the image
pub const IMAGE_SIZE: usize = WIDTH as usize * HEIGHT as usize;

/// TODO
pub struct Screen {
    pixels: [Pixel; IMAGE_SIZE],
}

impl Screen {
    /// TODO
    pub fn new(pixels: [Pixel; IMAGE_SIZE]) -> Self {
        Self { pixels }
    }

    /// Draw the screen to the terminal
    pub fn draw(&self, mut out: impl Write) -> io::Result<()> {
        // https://sw.kovidgoyal.net/kitty/graphics-protocol/#the-graphics-escape-code
        const ESCAPE: &[u8] = b"\x1b";

        // Everything other than the escape code is ASCII
        out.write_all(ESCAPE)?;
        write!(out, "_Ga=T,f=24,s={WIDTH},v={HEIGHT},c=40,r=10;")?;

        // Read the pixels as raw bytes and encode as base64
        // SAFETY: TODO
        let pixel_bytes: [u8; IMAGE_SIZE * 3] =
            unsafe { mem::transmute(self.pixels) };
        let mut b64_writer = EncoderWriter::new(&mut out, &STANDARD);
        b64_writer.write_all(&pixel_bytes)?;
        drop(b64_writer);

        // Finish the escape code
        out.write_all(ESCAPE)?;
        write!(out, "\\")?;
        Ok(())
    }
}

/// RGB pixel
#[derive(Copy, Clone, Debug)]
#[repr(C)] // We treat this as raw bytes when sending pixels over
pub struct Pixel {
    red: u8,
    green: u8,
    blue: u8,
}

impl Pixel {
    pub fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }
}
