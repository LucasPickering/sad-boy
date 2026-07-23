//! Graphics bindings for the terminal

use base64::{engine::general_purpose::STANDARD, write::EncoderWriter};
use std::{
    io::{self, Stdout, Write},
    mem, slice,
};
use termion::screen::{AlternateScreen, IntoAlternateScreen};

/// Width of the screen in terminal columns
const WIDTH_TERM: u16 = 80;
/// TODO
const IMAGE_ID: ImageId = ImageId(99);
/// Terminal escape code to trigger graphics rendering
///
/// https://sw.kovidgoyal.net/kitty/graphics-protocol/#the-graphics-escape-code
const ESCAPE: &str = "\u{1b}";

/// Interface to draw to the terminal
///
/// This uses the [Kitty Terminal Graphics Protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/)
/// to draw to the terminal.
///
/// Internally, this uses a double buffer to minimize the amount of data written
/// out to the terminal. When each frame is drawn, it's diffed against the
/// previous frame and only differences are written out. The terminal retains
/// the unmodified pixels in the new frame.
pub struct Screen {
    out: ScreenOut,
    /// The frame most recently written to the screen
    ///
    /// Invariant: `len() == self.width * self.height`
    pixels_last: Box<[Color]>,
    /// The next frame to write to the screen
    ///
    /// Invariant: `len() == self.width * self.height`
    pixels_next: Box<[Color]>,
    width: u16,
    height: u16,
}

impl Screen {
    /// Initialize a new screen adapter with the given pixel dimensions
    pub fn new(width: u16, height: u16) -> Self {
        let len = (width * height) as usize;
        Self {
            out: ScreenOut::new(),
            // TODO explain
            pixels_last: vec![Color::WHITE; len].into_boxed_slice(),
            pixels_next: vec![Color::BLACK; len].into_boxed_slice(),
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
        self.pixels_next[index] = color;
    }

    /// Draw the current screen buffer to the terminal
    ///
    /// This will diff the current frame against the last frame. Any differences
    /// will be written out to the terminal.
    pub fn draw(&mut self) {
        // Diff the previous and next frames, building a series of contiguous
        // pixels that have changed
        // TODO join blocks
        let mut blocks = vec![];
        for (i, (old, new)) in
            self.pixels_last.iter().zip(&self.pixels_next).enumerate()
        {
            if old != new {
                let x = i as u16 % self.width;
                let y = i as u16 / self.width;
                blocks.push(PixelBlock {
                    x,
                    y,
                    pixels: &self.pixels_next[i..=i],
                });
            }
        }

        // TODO draw as we go in the diff
        self.out
            .write_message(Message::CreateImage { image: IMAGE_ID })
            .expect("TODO");
        for block in blocks {
            self.out
                .write_message(Message::Frame {
                    image: IMAGE_ID,
                    x: block.x,
                    y: block.y,
                    pixels: block.pixels,
                })
                .expect("TODO");
        }
        self.out
            .write_message(Message::Put { image: IMAGE_ID })
            .expect("TODO");

        /* // Create the image
        let result = Message::DrawImage {
            width: self.width,
            height: self.height,
            columns: WIDTH_TERM,
            pixels: &self.pixels_next,
        }
        .write(&mut self.out);
        if let Err(error) = result {
            error!(%error, "Error drawing to screen");
        } */

        // Move this frame to the back buffer
        mem::swap(&mut self.pixels_last, &mut self.pixels_next);
        // Reset to black. This may not be necessary because the GPU should
        // rewrite every pixel on every frame, so it could be optimized out
        self.pixels_next.fill(Color::default());
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
    const BLACK: Self = Self::new(0, 0, 0);
    const WHITE: Self = Self::new(255, 255, 255);

    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }
}

/// Wrapper for the terminal output channel
struct ScreenOut(AlternateScreen<Stdout>);

impl ScreenOut {
    fn new() -> Self {
        Self(io::stdout().into_alternate_screen().unwrap())
    }

    /// Write a graphics message to the output buffer
    fn write_message(&mut self, message: Message) -> io::Result<()> {
        write!(self.0, "{ESCAPE}_G")?;
        match message {
            Message::CreateImage { image: image_id } => {
                self.write_control(&[Control::ImageId(image_id)])?;
            }
            Message::DrawImage {
                width,
                height,
                columns,
                pixels,
            } => {
                self.write_control(&[
                    Control::Action(Action::TransmitDisplay),
                    Control::Format(Format::Rgb),
                    Control::Width(width),
                    Control::Height(height),
                    Control::Columns(columns),
                    Control::CursorMovement(false),
                ])?;
                self.write_pixels(pixels)?;
            }
            Message::Frame {
                image,
                x,
                y,
                pixels,
            } => {
                self.write_control(&[
                    Control::Action(Action::Frame),
                    Control::ImageId(image),
                    Control::X(x),
                    Control::Y(y),
                ])?;
                self.write_pixels(pixels)?;
            }
            Message::Put { image: image_id } => {
                self.write_control(&[
                    Control::Action(Action::Put),
                    Control::ImageId(image_id),
                ])?;
            }
        }
        write!(self.0, "{ESCAPE}\\")?;
        Ok(())
    }

    /// Write a series of control attributes to the output buffer
    fn write_control(&mut self, controls: &[Control]) -> io::Result<()> {
        for (i, control) in controls.iter().enumerate() {
            let terminator = if i < controls.len() - 1 { ',' } else { ';' };
            let key = control.key();
            // TODO this should be possible without allocation
            let value = control.value();
            write!(self.0, "{key}={value}{terminator}")?;
        }
        Ok(())
    }

    /// Write a series of 24-bit colors as a base64 payload
    fn write_pixels(&mut self, pixels: &[Color]) -> Result<(), io::Error> {
        let ptr: *const [Color] = &raw const *pixels;
        let pixel_bytes: &[u8] = unsafe {
            slice::from_raw_parts(ptr.cast(), mem::size_of_val(pixels))
        };
        let mut b64_writer = EncoderWriter::new(&mut self.0, &STANDARD);
        b64_writer.write_all(pixel_bytes)?;
        drop(b64_writer);
        Ok(())
    }
}

/// A segment of pixels to be transmitted to the screen
///
/// This is generated by diffing two frames. Each block is a contiguous series
/// of pixels that have changed. Each block will be written to the terminal in
/// a separate message.
#[derive(Debug)]
struct PixelBlock<'a> {
    /// x coordinate of the first pixel in the block
    x: u16,
    /// y coordinate of the first pixel in the block
    y: u16,
    /// Contiguous pixels to write
    ///
    /// This can overflow the current line, wrapping to the next line at the
    /// screen width boundary.
    pixels: &'a [Color],
}

/// A message for the terminal in the [Terminal Graphics Protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/#control-data-reference)
enum Message<'a> {
    /// Create an image without displaying it
    CreateImage { image: ImageId },
    DrawImage {
        width: u16,
        height: u16,
        columns: u16,
        pixels: &'a [Color],
    },
    /// Transmit animation frame data
    Frame {
        image: ImageId,
        x: u16,
        y: u16,
        pixels: &'a [Color],
    },
    /// Display a previously transmitted image to the screen
    Put { image: ImageId },
}

/// A single control attribute in a graphics protocol message
///
/// https://sw.kovidgoyal.net/kitty/graphics-protocol/#control-data-reference
#[derive(Clone, Copy)]
enum Control {
    /// `a`: What action should the terminal take?
    Action(Action),
    /// `f`: Image format
    Format(Format),
    /// `i`: Image ID number
    ImageId(ImageId),
    /// `s`: Pixel width of the image
    Width(u16),
    /// `v`: Pixel height of the image
    Height(u16),
    /// `x`: Pixel x coordinate
    X(u16),
    /// `y`: Pixel y coordinate
    Y(u16),
    /// `c`: Number of columns to display the image over
    Columns(u16),
    /// `C`: Enable/disable cursor movement after placing the image
    CursorMovement(bool),
}

impl Control {
    fn key(self) -> char {
        match self {
            Control::Action(_) => 'a',
            Control::Format(_) => 'f',
            Control::ImageId(_) => 'i',
            Control::Width(_) => 's',
            Control::Height(_) => 'v',
            Control::X(_) => 'x',
            Control::Y(_) => 'y',
            Control::Columns(_) => 'c',
            Control::CursorMovement(_) => 'C',
        }
    }

    fn value(self) -> String {
        match self {
            Control::Action(Action::TransmitDisplay) => "T".into(),
            Control::Action(Action::Frame) => "f".into(),
            Control::Action(Action::Put) => "p".into(),
            Control::Format(Format::Rgb) => "24".into(),
            Control::CursorMovement(false) => "1".into(),
            Control::CursorMovement(true) => "0".into(),
            Control::ImageId(id) => id.0.to_string(),
            Control::Width(v)
            | Control::Height(v)
            | Control::X(v)
            | Control::Y(v)
            | Control::Columns(v) => v.to_string(),
        }
    }
}

/// Values for [Control::Action]
#[derive(Clone, Copy)]
enum Action {
    /// `a=T`: Transmit image and display it
    TransmitDisplay,
    /// `a=f`: Transmit animation frame data
    Frame,
    /// `a=p`: Display a previously transmitted image
    Put,
}

/// Values for [Control::Format]
#[derive(Clone, Copy)]
enum Format {
    /// `f=24`: 24-bit RGB format
    Rgb,
}

/// TODO
#[derive(Clone, Copy)]
struct ImageId(u16);
