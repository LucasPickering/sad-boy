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
    fmt::{self, Display},
    io::{self, Write},
    mem,
    num::NonZero,
};

/// Width of the screen in terminal columns
const WIDTH_TERM: u16 = 80;
/// Name for the POSIX shared memory block that holds frame data
const SHM_NAME: &str = "/sad_boy_shm";
/// Terminal escape code to trigger graphics rendering
///
/// https://sw.kovidgoyal.net/kitty/graphics-protocol/#the-graphics-escape-code
const ESCAPE: &str = "\u{1b}";

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

    #[cfg(test)]
    pub fn pixels(&self) -> &[Color] {
        &self.pixels
    }

    #[cfg(test)]
    pub fn width(&self) -> u16 {
        self.width
    }

    #[cfg(test)]
    pub fn height(&self) -> u16 {
        self.height
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

/// Draw a frame to the terminal
pub fn draw_frame(
    frame: &FrameBuffer,
    move_cursor: bool,
    mut out: impl io::Write,
) -> io::Result<()> {
    let pixels = &frame.pixels;
    // Sanity check
    debug_assert_eq!(
        pixels.len(),
        (frame.width as usize) * (frame.height as usize),
        "Pixel length must equal width*height"
    );

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
        out,
        SHM_NAME, // Payload = shared memory name
        // https://sw.kovidgoyal.net/kitty/graphics-protocol/#control-data-reference
        a = 'T',                    // action = Transmit + draw image
        f = 24,                     // format = RGB
        s = frame.width,            // pixel width
        v = frame.height,           // pixel height
        c = WIDTH_TERM,             // terminal width
        C = u8::from(!move_cursor), // enable/disable cursor movement
        t = 's',                    // transmit via shared memory
        S = len,                    // shared memory length
    )
}
