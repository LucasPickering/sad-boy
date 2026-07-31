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
};
use termion::{
    cursor,
    screen::{AlternateScreen, IntoAlternateScreen},
};
use tracing::error;

/// Width of the screen in terminal columns
const WIDTH_TERM: u16 = 80;
/// Name for the POSIX shared memory block that holds frame data
const SHM_NAME: &str = "/sad_boy_shm";
/// Terminal escape code to trigger graphics rendering
///
/// https://sw.kovidgoyal.net/kitty/graphics-protocol/#the-graphics-escape-code
const ESCAPE: &str = "\u{1b}";

/// Write a graphics message to the given output (probably stdout)
macro_rules! write_message {
    ($out:expr, $payload:expr, $($key:ident = $value:expr),* $(,)?) => {{
        write!($out, "{ESCAPE}_G")?;

        // Control args are comma-separated with a semicolon at the end
        let args = [
            $(format_args!("{}={}", stringify!($key), $value),)*
        ];
        for (i, arg) in args.iter().enumerate() {
            let terminator = if i < args.len() - 1 { ',' } else { ';' };
            write!($out, "{arg}{terminator}")?;
        }

        // Payload is encoded as base64
        // TODO if this is the only methodology long-term, pre-encode it and
        // remove the base64 dep
        let mut b64_writer = EncoderWriter::new(&mut $out, &STANDARD);
        b64_writer.write_all($payload.as_bytes())?;
        drop(b64_writer);

        write!($out, "{ESCAPE}\\")
    }};
}

/// An interface for a screen that can be drawn to
///
/// This is an abstraction over screen hardware. The backend could be a
/// terminal, web browser, etc.
pub trait Screen {
    /// Set the color value of a single pixel
    ///
    /// This will update the screen's internal frame buffer, but will not push
    /// anything to the visible screen yet. Panics if the pixel is out of
    /// bounds.
    fn set(&mut self, x: u16, y: u16, color: Color);

    /// Draw the current screen buffer to the terminal
    ///
    /// This will diff the current frame against the last frame. Any differences
    /// will be written out to the terminal.
    fn draw(&mut self);
}

/// A [Screen] implementation to draw to the terminal
///
/// This uses the [Kitty Terminal Graphics Protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/)
/// to draw to the terminal.
pub struct TerminalScreen {
    out: AlternateScreen<Stdout>,
    /// The next frame to write to the screen
    ///
    /// Invariant: `len() == self.width * self.height`
    pixels: Box<[Color]>,
    width: u16,
    height: u16,
}

impl TerminalScreen {
    /// Initialize a new screen adapter with the given pixel dimensions
    pub fn new(width: u16, height: u16) -> io::Result<Self> {
        let len = (width * height) as usize;

        let mut out = io::stdout().into_alternate_screen()?;
        // Move the cursor to the top-left
        write!(out, "{}", cursor::Goto(1, 1))?;

        Ok(Self {
            out,
            pixels: vec![Color::BLACK; len].into_boxed_slice(),
            width,
            height,
        })
    }

    fn draw_inner(&mut self) -> io::Result<()> {
        let pixels = &*self.pixels;

        // Use POSIX shared memory to pass the pixel data to the terminal. This
        // is (supposedly) much faster than writing to stdout
        let len = mem::size_of_val(pixels);
        let _ = nix::sys::mman::shm_unlink(SHM_NAME);
        let fd = nix::sys::mman::shm_open(
            SHM_NAME,
            OFlag::O_RDWR | OFlag::O_CREAT | OFlag::O_EXCL,
            Mode::S_IRUSR | Mode::S_IWUSR,
        )?;
        nix::unistd::ftruncate(&fd, len as i64)?;
        // SAFETY: TODO
        unsafe {
            let addr = nix::sys::mman::mmap(
                None,
                NonZero::new(len).unwrap(),
                ProtFlags::PROT_READ | ProtFlags::PROT_WRITE,
                MapFlags::MAP_SHARED,
                fd,
                0,
            )?;
            libc::memcpy(addr.as_ptr(), pixels.as_ptr().cast::<c_void>(), len);
        }

        write_message!(
            self.out,
            SHM_NAME, // Payload = shared memory name
            // https://sw.kovidgoyal.net/kitty/graphics-protocol/#control-data-reference
            a = 'T',         // action = Transmit + draw image
            f = 24,          // format = RGB
            s = self.width,  // pixel width
            v = self.height, // pixel height
            c = WIDTH_TERM,  // terminal width
            C = 1,           // disable cursor movement
            t = 's',         // transmit via shared memory
            S = len,         // shared memory length
        )
    }
}

impl Screen for TerminalScreen {
    fn set(&mut self, x: u16, y: u16, color: Color) {
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

    fn draw(&mut self) {
        if let Err(error) = self.draw_inner() {
            error!("Error drawing to screen: {error}");
        }
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

/// An in-memory screen for headless operation
#[cfg(test)]
#[derive(Debug)]
pub struct HeadlessScreen {
    /// The next frame to write to the screen
    ///
    /// Invariant: `len() == self.width * self.height`
    pixels: Box<[Color]>,
    width: u16,
    height: u16,
}

#[cfg(test)]
impl HeadlessScreen {
    pub fn new(width: u16, height: u16) -> Self {
        let len = (width * height) as usize;
        Self {
            pixels: vec![Color::BLACK; len].into_boxed_slice(),
            width,
            height,
        }
    }

    /// Get the screen pixels
    pub fn pixels(&self) -> &[Color] {
        &self.pixels
    }
}

#[cfg(test)]
impl Screen for HeadlessScreen {
    fn set(&mut self, x: u16, y: u16, color: Color) {
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

    fn draw(&mut self) {}
}
