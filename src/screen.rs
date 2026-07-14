//! Graphics bindings for the terminal
//!
//! This uses the [Kitty Terminal Graphics Protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/).

use base64::{engine::general_purpose::STANDARD, write::EncoderWriter};
use std::{
    io::{self, Write},
    mem, slice,
};

/// Width of the screen in pixels
const WIDTH_PIXELS: u16 = 160;
/// Height of the screen in pixels
const HEIGHT_PIXELS: u16 = 144;
/// Total number of pixels in the image
const IMAGE_SIZE: usize = WIDTH_PIXELS as usize * HEIGHT_PIXELS as usize;
/// Width of the screen in terminal columns
const WIDTH_TERM: u16 = 80;

/// TODO
pub struct Screen {
    pixels: Box<[Pixel; IMAGE_SIZE]>,
}

impl Screen {
    /// TODO
    pub fn new(pixels: Box<[Pixel; IMAGE_SIZE]>) -> Self {
        Self { pixels }
    }

    /// TODO delete
    pub fn test() -> Self {
        let red = Pixel::new(255, 0, 0);
        let green = Pixel::new(0, 255, 0);
        let blue = Pixel::new(0, 0, 255);
        let pixels: Vec<Pixel> = (0..IMAGE_SIZE)
            .map(|i| match i % 3 {
                0 => red,
                1 => green,
                2 => blue,
                3.. => unreachable!(),
            })
            .collect();
        let pixels: Box<[Pixel; IMAGE_SIZE]> = pixels.try_into().unwrap();
        Self::new(pixels)
    }

    /// Draw the screen to the terminal
    pub fn draw(&self, mut out: impl Write) -> io::Result<()> {
        // https://sw.kovidgoyal.net/kitty/graphics-protocol/#the-graphics-escape-code
        const ESCAPE: &[u8] = b"\x1b";

        // Everything other than the escape code is ASCII
        out.write_all(ESCAPE)?;
        write!(
            out,
            "_Ga=T,f=24,s={WIDTH_PIXELS},v={HEIGHT_PIXELS},c={WIDTH_TERM};"
        )?;

        // Read the pixels as raw bytes and encode as base64
        debug_assert_eq!(mem::size_of::<Pixel>(), 3, "Pixel must be 3 bytes");
        let ptr: *const [Pixel] = &raw const *self.pixels;
        // SAFETY:
        // - Pointer is always valid because we construct it safely above
        // - Length is correct because we're casting to BYTES, and it's just the
        //   number of items * bytes per item
        let pixel_bytes: &[u8] = unsafe {
            slice::from_raw_parts(
                ptr.cast(),
                self.pixels.len() * mem::size_of::<Pixel>(),
            )
        };
        let mut b64_writer = EncoderWriter::new(&mut out, &STANDARD);
        b64_writer.write_all(pixel_bytes)?;
        drop(b64_writer);

        // Finish the escape code
        out.write_all(ESCAPE)?;
        write!(out, "\\")?;
        Ok(())
    }
}

/// RGB pixel
#[derive(Clone, Copy, Debug)]
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
