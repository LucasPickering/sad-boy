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
    out: ScreenOut,
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
        let out = ScreenOut::new()?;
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
        self.out.write_message(
            // TODO this should be possible without allocation?
            &[
                // https://sw.kovidgoyal.net/kitty/graphics-protocol/#control-data-reference
                ('a', "T"),  // action = Transmit + draw image
                ('f', "24"), // format = RGB
                ('s', &self.width.to_string()), // pixel width
                ('v', &self.height.to_string()), // pixel height
                ('c', &WIDTH_TERM.to_string()), // terminal width
                ('C', "1"),  // disable cursor movement
                ('t', "s"),  // transmit via shared memory
                ('S', &len.to_string()), // shared memory length
            ],
            SHM_NAME.as_bytes(), // payload = shared memory name
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
    const BLACK: Self = Self::new(0, 0, 0);

    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }
}

/// Wrapper for the terminal output channel
struct ScreenOut(AlternateScreen<Stdout>);

impl ScreenOut {
    /// Initialize the terminal
    fn new() -> io::Result<ScreenOut> {
        let mut stdout = io::stdout().into_alternate_screen()?;
        // Move the cursor to the top-left
        write!(stdout, "{}", cursor::Goto(1, 1))?;
        Ok(Self(stdout))
    }

    /// Write a graphics message to the output buffer
    fn write_message(
        &mut self,
        controls: &[(char, &str)],
        payload: &[u8],
    ) -> io::Result<()> {
        write!(self.0, "{ESCAPE}_G")?;
        for (i, (key, value)) in controls.iter().enumerate() {
            let terminator = if i < controls.len() - 1 { ',' } else { ';' };
            write!(self.0, "{key}={value}{terminator}")?;
        }

        // Payload is encoded as base64
        let mut b64_writer = EncoderWriter::new(&mut self.0, &STANDARD);
        b64_writer.write_all(payload)?;
        drop(b64_writer);

        write!(self.0, "{ESCAPE}\\")?;
        Ok(())
    }
}
