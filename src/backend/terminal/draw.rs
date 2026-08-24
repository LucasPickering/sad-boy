//! Terminal graphics drawing
//!
//! This uses the Kitty Terminal Graphics Protocol to draw pretty pictures to
//! a medium primarily known for text.
//!
//! https://sw.kovidgoyal.net/kitty/graphics-protocol/

use crate::backend::FrameBuffer;
use base64::{engine::general_purpose::STANDARD, write::EncoderWriter};
use crossterm::cursor;
use nix::{
    fcntl::OFlag,
    libc,
    sys::{
        mman::{MapFlags, ProtFlags},
        stat::Mode,
    },
};
use ratatui::layout::Rect;
use std::{
    ffi::c_void,
    io::{self, Write},
    mem,
    num::NonZero,
    sync::atomic::{AtomicUsize, Ordering},
};

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
        let mut b64_writer = EncoderWriter::new(&mut $out, &STANDARD);
        b64_writer.write_all($payload)?;
        drop(b64_writer);

        write!($out, "{ESCAPE}\\")
    }};
}

/// Draw a frame to the terminal
///
/// ## Params
/// - `frame`: Frame buffer holding the pixels to draw
/// - `size`: Terminal size to draw to (location is the current cursor)
/// - `move_cursor`: Should the cursor move to the end of the frame?
/// - `out`: Output channel (probably stdout)
pub fn draw_frame(
    frame: &FrameBuffer,
    location: FrameLocation,
    mut out: impl io::Write,
) -> io::Result<()> {
    // Each frame needs a unique ID to prevent them from overwriting each other.
    // This is (hopefully) not an issue during normal emulation, but can be in
    // tests.
    static FRAME_ID: AtomicUsize = AtomicUsize::new(0);
    let shm_name =
        format!("/sad_boy_shm{}", FRAME_ID.fetch_add(1, Ordering::Relaxed));

    let pixels = frame.pixels();
    // Sanity checks
    debug_assert_eq!(
        pixels.len(),
        (frame.width() as usize) * (frame.height() as usize),
        "Pixel length must equal width*height"
    );

    // Use POSIX shared memory to pass the pixel data to the terminal. This
    // is (supposedly) much faster than writing to stdout
    // https://sw.kovidgoyal.net/kitty/graphics-protocol/#local-client
    let len = mem::size_of_val(pixels);
    let _ = nix::sys::mman::shm_unlink(shm_name.as_str());
    let fd = nix::sys::mman::shm_open(
        shm_name.as_str(),
        OFlag::O_RDWR | OFlag::O_CREAT | OFlag::O_EXCL,
        Mode::S_IRUSR | Mode::S_IWUSR,
    )?;
    nix::unistd::ftruncate(&fd, len as i64)?;
    // SAFETY: Alright so I'm guessing a bit here because the Rust docs for
    // nix/libc don't list *specifically* what's unsafe about these.
    // - Page length is the BYTE length of the pixel slice, established above
    // - memcpy() source is the pointer to that pixel length
    // Seems safe enough to me!
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

    let (width, height, cursor_adv) = match location {
        FrameLocation::Fixed(area) => {
            write!(out, "{}", cursor::MoveTo(area.x, area.y))?;
            (area.width, area.height, 1) // 1 = don't advance cursor
        }
        #[cfg(test)]
        FrameLocation::Auto(size) => {
            (size.width, size.height, 0) // 0 = advance cursor
        }
    };
    write_message!(
        out,
        shm_name.as_bytes(), // Payload = shared memory name
        // https://sw.kovidgoyal.net/kitty/graphics-protocol/#control-data-reference
        a = 'T',            // action = Transmit + draw image
        f = 24,             // format = RGB
        s = frame.width(),  // pixel width
        v = frame.height(), // pixel height
        c = width,          // width in terminal columns
        r = height,         // height in terminal rows
        C = cursor_adv,     // enable/disable cursor movement
        t = 's',            // transmit via shared memory
        S = len,            // shared memory length
    )?;
    out.flush()
}

/// Define where a frame should be drawn on the screen in [draw_frame]
pub enum FrameLocation {
    /// Draw the frame with a specific position and size
    ///
    /// The cursor will be moved to this location and will remain there
    /// afterward (it will not be advanced to the end of the frame).
    Fixed(Rect),
    /// Draw the frame with a fixed size at the current cursor location
    ///
    /// The cursor will be advanced to the end of the frame. Use this for
    /// inline printing (e.g. in assertion output).
    #[cfg(test)]
    Auto(ratatui::layout::Size),
}
