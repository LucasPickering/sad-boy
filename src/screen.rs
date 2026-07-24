//! Graphics bindings for the terminal

use base64::{engine::general_purpose::STANDARD, write::EncoderWriter};
use nix::{
    fcntl::OFlag,
    libc,
    sys::{
        mman::{MapFlags, ProtFlags},
        stat::Mode,
    },
};
use std::{
    ffi::c_void,
    io::{self, Stdout, Write},
    mem,
    num::NonZero,
    slice,
};
use termion::{
    cursor,
    screen::{AlternateScreen, IntoAlternateScreen},
};

/// Width of the screen in terminal columns
const WIDTH_TERM: u16 = 80;
/// TODO
const SHM_NAME: &str = "/sad_boy_shm";
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
    /// The next frame to write to the screen
    ///
    /// Invariant: `len() == self.width * self.height`
    pixels: Box<[Color]>,
    width: u16,
    height: u16,
}

impl Screen {
    /// Initialize a new screen adapter with the given pixel dimensions
    pub fn new(width: u16, height: u16) -> Self {
        let len = (width * height) as usize;
        let out = ScreenOut(io::stdout().into_alternate_screen().unwrap());
        Self {
            out,
            pixels: vec![Color::BLACK; len].into_boxed_slice(),
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

    /// Draw the current screen buffer to the terminal
    ///
    /// This will diff the current frame against the last frame. Any differences
    /// will be written out to the terminal.
    pub fn draw(&mut self) {
        let pixels = &*self.pixels;
        self.out.set_cursor(1, 1);

        // TODO
        let len = mem::size_of_val(pixels);
        nix::sys::mman::shm_unlink(SHM_NAME);
        let fd = nix::sys::mman::shm_open(
            SHM_NAME,
            OFlag::O_RDWR | OFlag::O_CREAT | OFlag::O_EXCL,
            Mode::S_IRUSR | Mode::S_IWUSR,
        )
        .unwrap(); // TODO
        nix::unistd::ftruncate(&fd, len as i64).unwrap();
        unsafe {
            let addr = nix::sys::mman::mmap(
                None,
                NonZero::new(len).unwrap(),
                ProtFlags::PROT_READ | ProtFlags::PROT_WRITE,
                MapFlags::MAP_SHARED,
                fd,
                0,
            )
            .expect("mmap");
            libc::memcpy(addr.as_ptr(), pixels.as_ptr().cast::<c_void>(), len);
        }
        self.out
            .write_message(Message::Any {
                control: &[
                    Control::Action(Action::TransmitDisplay),
                    Control::Format(Format::Rgb),
                    Control::Width(self.width),
                    Control::Height(self.height),
                    Control::Columns(WIDTH_TERM),
                    Control::CursorMovement(false),
                    Control::Any('t', "s".into()),
                    Control::Any('S', len.to_string()),
                ],
                payload: SHM_NAME.as_bytes(),
            })
            .expect("TODO");
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

    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }
}

/// Wrapper for the terminal output channel
struct ScreenOut(AlternateScreen<Stdout>);

impl ScreenOut {
    /// Move the terminal cursor to the **1-based** location, starting from the
    /// top-left
    fn set_cursor(&mut self, x: u16, y: u16) -> io::Result<()> {
        write!(self.0, "{}", cursor::Goto(x, y))
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
            Message::Any { control, payload } => {
                self.write_control(control)?;
                self.write_payload(payload)?;
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
    fn write_pixels(&mut self, pixels: &[Color]) -> io::Result<()> {
        let ptr: *const [Color] = &raw const *pixels;
        let pixel_bytes: &[u8] = unsafe {
            slice::from_raw_parts(ptr.cast(), mem::size_of_val(pixels))
        };
        self.write_payload(pixel_bytes)?;
        Ok(())
    }

    /// Write a base64-encoded binary payload
    fn write_payload(&mut self, payload: &[u8]) -> io::Result<()> {
        let mut b64_writer = EncoderWriter::new(&mut self.0, &STANDARD);
        b64_writer.write_all(payload)?;
        Ok(())
    }
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
    /// TODO
    Any {
        control: &'a [Control],
        payload: &'a [u8],
    },
}

/// A single control attribute in a graphics protocol message
///
/// https://sw.kovidgoyal.net/kitty/graphics-protocol/#control-data-reference
#[derive(Clone)]
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
    /// TODO
    Any(char, String),
}

impl Control {
    fn key(&self) -> char {
        match self {
            Self::Action(_) => 'a',
            Self::Format(_) => 'f',
            Self::ImageId(_) => 'i',
            Self::Width(_) => 's',
            Self::Height(_) => 'v',
            Self::X(_) => 'x',
            Self::Y(_) => 'y',
            Self::Columns(_) => 'c',
            Self::CursorMovement(_) => 'C',
            Self::Any(key, _) => *key,
        }
    }

    fn value(&self) -> String {
        match self {
            Self::Action(Action::TransmitDisplay) => "T".into(),
            Self::Action(Action::Frame) => "f".into(),
            Self::Action(Action::Put) => "p".into(),
            Self::Format(Format::Rgb) => "24".into(),
            Self::CursorMovement(false) => "1".into(),
            Self::CursorMovement(true) => "0".into(),
            Self::ImageId(id) => id.0.to_string(),
            Self::Width(v)
            | Self::Height(v)
            | Self::X(v)
            | Self::Y(v)
            | Self::Columns(v) => v.to_string(),
            Self::Any(_, v) => v.clone(),
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
