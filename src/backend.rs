//! Hardware abstractions (input and screens)
//!
//! This module is emulator-agnostic. The backend can be used to read input
//! and draw output for any emulated system.

mod terminal;

pub use terminal::TerminalBackend;

use crate::{emu::DebugInfo, input::InputEvent};
use std::fmt::{self, Display};

/// An interface for a screen and input
///
/// This is an abstraction over hardware. The backend could be a terminal, web
/// browser, etc.
pub trait Backend {
    /// TODO
    fn debug(&mut self, info: &DebugInfo);

    /// Draw the given frame buffer to the terminal
    fn draw(&mut self, frame: &FrameBuffer);

    /// Get the next queued input event
    ///
    /// Return `None` if no inputs are pending.
    fn next_event(&mut self) -> Option<InputEvent>;

    /// Get the next queued input event, blocking until an event is available
    fn next_event_blocking(&mut self) -> InputEvent;

    /// Should the emulator exit?
    ///
    /// This is called by the emulator on each tick and should be used to
    /// monitor exit conditions (such as process signals).
    ///
    /// This should *not* check for a [InputEvent::Quit] input. That will be
    /// monitored separately via [Self::next].
    fn should_quit(&self, debug_info: &DebugInfo) -> bool;
}

/// An in-memory [Backend] for testing and headless operation
pub struct HeadlessBackend {
    /// Most recent drawn frame
    last_frame: Option<FrameBuffer>,
    /// Callback to check if the app should quit
    ///
    /// Tests use this to define custom termination conditions.
    should_quit: Box<dyn Fn(&DebugInfo) -> bool>,
}

impl HeadlessBackend {
    pub fn new(should_quit: impl 'static + Fn(&DebugInfo) -> bool) -> Self {
        Self {
            last_frame: None,
            should_quit: Box::new(should_quit),
        }
    }

    /// Assert that the screen pixels match the given pixel array
    #[cfg(test)]
    #[track_caller]
    pub fn assert_pixels(&self, expected: &FrameBuffer) {
        use std::fmt::Write;

        struct Mismatch {
            x: u16,
            y: u16,
            actual: Color,
            expected: Color,
        }

        let actual = self
            .last_frame
            .as_ref()
            .expect("Screen has not been drawn to");
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
                writeln!(messages, "At ({x},{y}): {actual} != {expected}")
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
    #[cfg(test)]
    fn draw_pixels(&self, title: &str, frame: &FrameBuffer) {
        use std::io::Write;

        let mut stderr = std::io::stderr();
        writeln!(stderr, "{title}:").unwrap();
        terminal::draw_frame(frame, true, &mut stderr).unwrap();
        writeln!(stderr).unwrap();
    }
}

impl Backend for HeadlessBackend {
    fn debug(&mut self, _info: &DebugInfo) {}

    fn draw(&mut self, frame: &FrameBuffer) {
        self.last_frame = Some(frame.clone());
    }

    fn next_event(&mut self) -> Option<InputEvent> {
        None
    }

    fn next_event_blocking(&mut self) -> InputEvent {
        todo!()
    }

    fn should_quit(&self, debug_info: &DebugInfo) -> bool {
        (self.should_quit)(debug_info)
    }
}

/// In-memory buffer for a frame to be drawn
#[derive(Clone, Debug)]
pub struct FrameBuffer {
    /// Pixel data in column-major format
    ///
    /// Invariant: `len() == self.width * self.height`
    pixels: Box<[Color]>,
    /// Pixel width of the frame
    width: u16,
    /// Pixel height of the frame
    height: u16,
}

impl FrameBuffer {
    /// Initialize a new frame buffer
    pub fn new(width: u16, height: u16) -> Self {
        let len = (width * height) as usize;
        Self {
            pixels: vec![Color::BLACK; len].into_boxed_slice(),
            width,
            height,
        }
    }

    /// Initialize a new frame buffer with static contents for test assertions
    #[cfg(test)]
    pub fn test(width: u16, height: u16, pixels: Vec<Color>) -> Self {
        assert_eq!(
            pixels.len(),
            (width * height) as usize,
            "Pixel length must equal width*height"
        );
        Self {
            pixels: pixels.into_boxed_slice(),
            width,
            height,
        }
    }

    /// Get frame pixels as a slice
    pub fn pixels(&self) -> &[Color] {
        &self.pixels
    }

    /// Number of columns of pixels in the frame
    pub fn width(&self) -> u16 {
        self.width
    }

    /// Number of rows of pixels in the frame
    pub fn height(&self) -> u16 {
        self.height
    }

    /// Set the value of a single pixel
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
        self.pixels.fill(Color::BLACK);
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

    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }
}

impl Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{},{},{}", self.red, self.green, self.blue)
    }
}
