//! Hardware abstractions (input and screens)
//!
//! This module is emulator-agnostic. The backend can be used to read input
//! and draw output for any emulated system.

mod terminal;

pub use terminal::TerminalBackend;

use std::fmt::{self, Display};

/// An interface for a screen and input
///
/// This is an abstraction over hardware. It provides everything a Game Boy
/// needs to provide interaction with the user. The backend could be a terminal,
/// web browser, etc.
pub trait Backend {
    /// Draw the given frame buffer to the screen
    fn draw(&mut self, frame: &FrameBuffer);
}

/// An in-memory [Backend] for testing and headless operation
#[derive(Default)]
pub struct HeadlessBackend {
    /// Most recent drawn frame
    last_frame: Option<FrameBuffer>,
}

impl HeadlessBackend {
    pub fn new() -> Self {
        Self::default()
    }

    /// Assert that the most recently drawn frame matches the given one
    #[cfg(test)]
    #[track_caller]
    pub fn assert_pixels(&self, expected: &FrameBuffer) {
        let actual = self
            .last_frame
            .as_ref()
            .expect("Screen has not been drawn to");
        actual.assert_pixels(expected);
    }
}

impl Backend for HeadlessBackend {
    fn draw(&mut self, frame: &FrameBuffer) {
        self.last_frame = Some(frame.clone());
    }
}

/// In-memory buffer for a frame to be drawn
#[derive(Clone, Debug)]
pub struct FrameBuffer {
    /// Pixel data in column-major format
    ///
    /// Invariant: `len() == Self::WIDTH * Self::HEIGHT`
    pixels: Box<[Color]>,
}

impl FrameBuffer {
    /// Width of a frame in pixels
    pub const WIDTH: u16 = 160;
    /// Height of a frame in pixels
    pub const HEIGHT: u16 = 144;
    /// Number of pixels in a frame
    #[cfg(test)]
    pub const LENGTH: usize = Self::WIDTH as usize * Self::HEIGHT as usize;

    /// Initialize a new frame buffer
    pub fn new() -> Self {
        let len = (Self::WIDTH * Self::HEIGHT) as usize;
        Self {
            pixels: vec![Color::BLACK; len].into_boxed_slice(),
        }
    }

    /// Get frame pixels as a slice
    pub fn pixels(&self) -> &[Color] {
        &self.pixels
    }

    /// Number of columns of pixels in the frame
    pub fn width(&self) -> u16 {
        Self::WIDTH
    }

    /// Number of rows of pixels in the frame
    pub fn height(&self) -> u16 {
        Self::HEIGHT
    }

    /// Set the value of a single pixel
    pub fn set(&mut self, x: u16, y: u16, color: Color) {
        assert!(
            x < Self::WIDTH,
            "x {x} must be less than width {width}",
            width = Self::WIDTH
        );
        assert!(
            y < Self::HEIGHT,
            "y {y} must be less than height {height}",
            height = Self::HEIGHT
        );
        self.pixels[self.index(x, y)] = color;
    }

    /// Reset all pixels to black
    pub fn reset(&mut self) {
        self.pixels.fill(Color::BLACK);
    }

    fn index(&self, x: u16, y: u16) -> usize {
        y as usize * Self::WIDTH as usize + x as usize
    }
}

#[cfg(test)]
impl FrameBuffer {
    /// Initialize a new frame buffer with static contents for test assertions
    #[cfg(test)]
    pub fn from_pixels(pixels: Vec<Color>) -> Self {
        assert_eq!(
            pixels.len(),
            (Self::WIDTH * Self::HEIGHT) as usize,
            "Pixel length must equal width*height"
        );
        Self {
            pixels: pixels.into_boxed_slice(),
        }
    }

    /// Set all pixels in a given area to a color
    pub fn set_region(
        &mut self,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
        color: Color,
    ) {
        let left = x;
        let right = x + width;
        let top = y;
        let bottom = y + height;
        // We can write each row as a chunk
        for y in top..bottom {
            let start = self.index(left, y);
            let end = self.index(right, y);
            self.pixels[start..end].fill(color);
        }
    }

    /// Assert that the screen pixels match the given pixel array
    #[track_caller]
    pub fn assert_pixels(&self, expected: &FrameBuffer) {
        use std::fmt::Write;

        struct Mismatch {
            x: u16,
            y: u16,
            actual: Color,
            expected: Color,
        }

        let actual = self;
        assert_eq!(
            actual.pixels().len(),
            expected.pixels().len(),
            "Expected pixel array must be length {} * {}",
            actual.width(),
            actual.height()
        );

        // Find mismatched pixels
        let mismatched: Vec<Mismatch> = actual
            .pixels()
            .iter()
            .zip(expected.pixels())
            .enumerate()
            .filter_map(|(i, (color_actual, color_expected))| {
                if color_actual == color_expected {
                    None
                } else {
                    let i = i as u16;
                    let x = i % actual.width();
                    let y = i / actual.width();
                    Some(Mismatch {
                        x,
                        y,
                        actual: *color_actual,
                        expected: *color_expected,
                    })
                }
            })
            .collect();

        if !mismatched.is_empty() {
            // Print the screens
            self.draw_pixels("Actual", actual);
            self.draw_pixels("Expected", expected);

            // Show mismatched cells, but cap it to prevent absurd amounts of
            // output
            let mut messages = String::new();
            let truncated = mismatched.get(0..10).unwrap_or(&mismatched);
            for Mismatch {
                x,
                y,
                actual,
                expected,
            } in truncated
            {
                writeln!(messages, "At {x},{y}: {actual} != {expected}")
                    .unwrap();
            }
            if truncated.len() < mismatched.len() {
                let remaining = mismatched.len() - truncated.len();
                writeln!(messages, "...and {remaining} more").unwrap();
            }
            panic!("Screen mismatch:\n{messages}");
        }
    }

    /// Print a screen to stderr for an assertion
    fn draw_pixels(&self, title: &str, frame: &FrameBuffer) {
        use std::io::Write;

        let mut stderr = std::io::stderr();
        writeln!(stderr, "{title}:").unwrap();
        terminal::draw_frame(frame, true, &mut stderr).unwrap();
        writeln!(stderr).unwrap();
    }
}

/// 24-bit RGB color
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(C)] // We treat this as raw bytes when sending pixels over
pub struct Color {
    red: u8,
    green: u8,
    blue: u8,
}

impl Color {
    pub const BLACK: Self = Self::new(0, 0, 0);
    pub const DARK_GRAY: Color = Color::new(85, 85, 85);
    pub const LIGHT_GRAY: Color = Color::new(170, 170, 170);
    pub const WHITE: Color = Color::new(255, 255, 255);

    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }
}

impl Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{},{},{}", self.red, self.green, self.blue)
    }
}
