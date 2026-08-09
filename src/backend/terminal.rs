//! Hardware bindings for the terminal

use crate::{
    backend::{Backend, FrameBuffer},
    emu::{Cycles, DebugInfo, Instruction},
    input::InputEvent,
    util::IntDisplay,
};
use base64::{engine::general_purpose::STANDARD, write::EncoderWriter};
use nix::{
    fcntl::OFlag,
    libc,
    sys::{
        mman::{MapFlags, ProtFlags},
        stat::Mode,
    },
};
use signal_hook::consts::signal;
use std::{
    ffi::c_void,
    fmt::{self, Display},
    io::{self, Stdout, Write},
    mem,
    num::NonZero,
    panic,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
};
use termion::{
    clear, cursor,
    event::Key,
    input::TermRead,
    raw::{IntoRawMode, RawTerminal},
    screen::{AlternateScreen, IntoAlternateScreen, ToMainScreen},
};
use tracing::error;

/// Width of the screen in terminal columns
const TERM_WIDTH: u16 = 60;
/// Height of the screen in terminal rows
const TERM_HEIGHT: u16 = 20;
/// Terminal escape code to trigger graphics rendering
///
/// https://sw.kovidgoyal.net/kitty/graphics-protocol/#the-graphics-escape-code
const ESCAPE: &str = "\u{1b}";

/// A [Backend] implementation to draw to the terminal
///
/// This uses the [Kitty Terminal Graphics Protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/)
/// to draw to stdout. It reads input from stdin.
pub struct TerminalBackend {
    /// Queue of events to be handled
    ///
    /// A background thread listens for input events and pushes them into this
    /// queue. The main loop pops off the queue via [Self::next_event].
    input_rx: mpsc::Receiver<InputEvent>,
    /// Channel to write output to (stdout)
    out: AlternateScreen<RawTerminal<Stdout>>,
    /// Flag set by the signal handler when a termination signal is received
    quit: Arc<AtomicBool>,
}

impl TerminalBackend {
    /// Initialize a new terminal adapter with the given pixel dimensions
    ///
    /// This will also spawn a background thread to listen for quit signals
    /// from the OS.
    pub fn new() -> io::Result<Self> {
        Self::init_panic_hook()?;

        let mut out = io::stdout().into_raw_mode()?.into_alternate_screen()?;
        // Move the cursor to the top-left
        write!(out, "{}", cursor::Goto(1, 1))?;

        // Start a signal listener for SIGINT and friends.
        // We need to catch signals to allow the screen to clean up before exit.
        let quit = Arc::new(AtomicBool::new(false));
        let signals = [
            signal::SIGINT,
            signal::SIGHUP,
            signal::SIGQUIT,
            signal::SIGTERM,
        ];
        for signal in signals {
            signal_hook::flag::register(signal, quit.clone()).unwrap();
        }

        // Listen for input in a background thread. Termion only exposes a fully
        // blocking interator for input handling, so we can't access it in the
        // main thread. The background thread will put any relevant events in
        // the queue.
        let (input_tx, input_rx) = mpsc::channel();
        thread::spawn(move || Self::handle_input(input_tx));

        Ok(Self {
            input_rx,
            out,
            quit,
        })
    }

    /// Monitor stdin for input
    ///
    /// When a relevant event is received, it's pushed into the given channel.
    fn handle_input(input_tx: mpsc::Sender<InputEvent>) {
        for result in io::stdin().keys() {
            match result.map(Self::map_key) {
                Ok(Some(event)) => {
                    // If the channel is closed, we can just exit
                    if input_tx.send(event).is_err() {
                        break;
                    }
                }
                Ok(None) => {}
                Err(error) => error!("Error reading input: {error}"),
            }
        }
    }

    /// Map a key event to an [InputEvent]
    fn map_key(key: Key) -> Option<InputEvent> {
        match key {
            Key::Char(' ') => Some(InputEvent::DebugPauseToggle),
            Key::Right => Some(InputEvent::DebugStepNext),
            Key::Char('q') | Key::Ctrl('c') => Some(InputEvent::Quit),
            _ => None,
        }
    }

    /// Initialize a panic hook that will reset the terminal state on panic
    ///
    /// The termion output wrappers will reset termianl state on _drop_, but
    /// drop handlers aren't called on panic.
    fn init_panic_hook() -> io::Result<()> {
        let raw_output = io::stdout().into_raw_mode()?;
        let original_hook = panic::take_hook();
        panic::set_hook(Box::new(move |panic_info| {
            // intentionally ignore errors here since we're already in a panic
            let _ = raw_output.suspend_raw_mode();
            let _ = write!(io::stdout(), "{ToMainScreen}");
            let _ = io::stdout().flush();
            original_hook(panic_info);
        }));
        Ok(())
    }

    /// Write debug info to the terminal
    fn write_debug(&mut self, lines: &[fmt::Arguments]) -> io::Result<()> {
        // Write each line, starting at the bottom of the emulator screen
        for (line, y) in lines.iter().zip(TERM_HEIGHT + 1..) {
            // Terminal is in raw mode so we have to move the cursor and clear
            // the line manually
            write!(
                self.out,
                "{goto}{clear}{line}",
                goto = cursor::Goto(1, y),
                clear = clear::CurrentLine,
            )?;
        }
        // Reset cursor back to the start
        write!(self.out, "{}", cursor::Goto(1, 1))?;
        self.out.flush()
    }
}

impl Backend for TerminalBackend {
    fn debug(&mut self, info: &DebugInfo) {
        struct Reg<T>(T);

        impl Display for Reg<u8> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(
                    f,
                    "{v} ({hex}, {bin})",
                    v = self.0,
                    hex = IntDisplay::hex(self.0),
                    bin = IntDisplay::binary(self.0),
                )
            }
        }

        impl Display for Reg<u16> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(
                    f,
                    "{v} ({hex})",
                    v = self.0,
                    hex = IntDisplay::hex(self.0),
                )
            }
        }

        let cpu = &info.cpu;
        let (prev_instruction, prev_cycles, prev_bytes) = cpu
            .previous_instruction
            .unwrap_or((Instruction::Invalid, Cycles(0), 0));
        let (next_instruction, next_cycles, next_bytes) = cpu.next_instruction;
        let result = self.write_debug(&[
            format_args!("Clock: {} cy", info.clock_cycles),
            format_args!("=== CPU ==="),
            format_args!(
                "Prev: {prev_instruction} ({prev_cycles} cy, {prev_bytes} B)"
            ),
            format_args!(
                "Next: {next_instruction} ({next_cycles} cy, {next_bytes} B)"
            ),
            // Registers
            format_args!("a: {}", Reg(cpu.a)),
            format_args!(
                "f: {} {}",
                IntDisplay::hex(cpu.f.as_u8()),
                cpu.f.unpack()
            ),
            format_args!("af: {}", Reg(cpu.af)),
            format_args!("b: {}", Reg(cpu.b)),
            format_args!("c: {}", Reg(cpu.c)),
            format_args!("bc: {}", Reg(cpu.bc)),
            format_args!("d: {}", Reg(cpu.d)),
            format_args!("e: {}", Reg(cpu.e)),
            format_args!("de: {}", Reg(cpu.de)),
            format_args!("h: {}", Reg(cpu.h)),
            format_args!("l: {}", Reg(cpu.l)),
            format_args!("hl: {}", Reg(cpu.hl)),
            format_args!("pc: {}", cpu.pc),
            format_args!("sp: {}", cpu.sp),
            format_args!(
                "Interrupts: {}",
                if cpu.interrupts_enabled {
                    "enabled"
                } else {
                    "disabled"
                }
            ),
        ]);
        if let Err(error) = result {
            error!("Error writing debug info to terminal: {error}");
        }
    }

    fn draw(&mut self, frame: &FrameBuffer) {
        if let Err(error) = draw_frame(frame, false, &mut self.out) {
            error!("Error drawing to terminal: {error}");
        }
    }

    fn next_event(&mut self) -> Option<InputEvent> {
        // Grab the next event off the queue
        match self.input_rx.try_recv() {
            Ok(event) => Some(event),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => todo!(),
        }
    }

    fn next_event_blocking(&mut self) -> InputEvent {
        // Grab the next event off the queue
        self.input_rx.recv().expect("TODO")
    }

    fn should_quit(&self, _debug_info: &DebugInfo) -> bool {
        self.quit.load(Ordering::Relaxed)
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
        b64_writer.write_all($payload)?;
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
    static FRAME_ID: AtomicUsize = AtomicUsize::new(0);
    // Each frame needs a unique ID to prevent them for overwriting each other.
    // This is (hopefully) not an issue during normal emulation, but can be in
    // tests.
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
    let len = mem::size_of_val(pixels);
    let _ = nix::sys::mman::shm_unlink(shm_name.as_str());
    let fd = nix::sys::mman::shm_open(
        shm_name.as_str(),
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
        shm_name.as_bytes(), // Payload = shared memory name
        // https://sw.kovidgoyal.net/kitty/graphics-protocol/#control-data-reference
        a = 'T',                    // action = Transmit + draw image
        f = 24,                     // format = RGB
        s = frame.width(),          // pixel width
        v = frame.height(),         // pixel height
        c = TERM_WIDTH,             // width in terminal columns
        r = TERM_HEIGHT,            // height in terminal rows
        C = u8::from(!move_cursor), // enable/disable cursor movement
        t = 's',                    // transmit via shared memory
        S = len,                    // shared memory length
    )?;
    out.flush()
}
