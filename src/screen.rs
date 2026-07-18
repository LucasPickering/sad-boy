//! Graphics bindings for the terminal
//!
//! This uses the [Kitty Terminal Graphics Protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/).

use base64::{engine::general_purpose::STANDARD, write::EncoderWriter};
use std::{
    io::{self, Write},
    mem, slice,
};

/// Width of the screen in terminal columns
const WIDTH_TERM: u16 = 80;

/// TODO
pub struct Screen {
    pixels: Box<[Color]>,
    width: u16,
    height: u16,
}

impl Screen {
    /// TODO
    pub fn new(width: u16, height: u16) -> Self {
        let len = (width * height) as usize;
        Self {
            pixels: vec![Color::default(); len].into_boxed_slice(),
            width,
            height,
        }
    }

    /// Set the color value of a single pixel
    ///
    /// Panics if the pixel is out of bounds.
    pub fn set(&mut self, x: u16, y: u16, color: Color) {
        assert!(
            x < self.width,
            "x {x} must be less than width {width}",
            width = self.width
        );
        assert!(
            y < self.height,
            "y {y} must be less than height {height}",
            height = self.height
        );
        let index = (y * self.width + x) as usize;
        self.pixels[index] = color;
    }

    /// Reset all pixels to black
    pub fn reset(&mut self) {
        self.pixels.fill(Color::default());
    }

    /// Draw the current screen buffer to the terminal
    pub fn draw(&self) -> io::Result<()> {
        // https://sw.kovidgoyal.net/kitty/graphics-protocol/#the-graphics-escape-code
        const ESCAPE: &[u8] = b"\x1b";
        let mut out = io::stdout();

        // Everything other than the escape code is ASCII
        out.write_all(ESCAPE)?;
        write!(
            out,
            "_Ga=T,f=24,s={width},v={height},c={WIDTH_TERM};",
            width = self.width,
            height = self.height
        )?;

        // Cast the pixels to raw bytes
        let ptr: *const [Color] = &raw const *self.pixels;
        // SAFETY:
        // - Pointer is always valid because we construct it safely above
        // - Length is correct because we're casting to BYTES, and it's just the
        //   number of items * bytes per item
        let pixel_bytes: &[u8] = unsafe {
            slice::from_raw_parts(
                ptr.cast(),
                self.pixels.len() * mem::size_of::<Color>(),
            )
        };
        // Encode and write as base64
        let mut b64_writer = EncoderWriter::new(&mut out, &STANDARD);
        b64_writer.write_all(pixel_bytes)?;
        drop(b64_writer);

        // Finish the escape code
        out.write_all(ESCAPE)?;
        write!(out, "\\")?;
        Ok(())
    }
}

/// 24-bit RGB color
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)] // We treat this as raw bytes when sending pixels over
pub struct Color {
    red: u8,
    green: u8,
    blue: u8,
}

impl Color {
    pub fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }
}
