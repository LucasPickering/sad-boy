//! Hardware bindings for the terminal

use crate::{
    Debugger,
    backend::{Backend, FrameBuffer},
    emu::{Cpu, Cycles, GameBoy, InstructionInfo, instruction::Instruction},
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
use ratatui::{
    Terminal,
    layout::{Constraint, Layout, Rect},
    prelude::{Buffer, TermionBackend},
    symbols::merge::MergeStrategy,
    text::Text,
    widgets::{Block, BorderType, Borders, Widget},
};
use signal_hook::consts::signal;
use std::{
    ffi::c_void,
    fmt::{self, Display},
    io::{self, Stdout, Write},
    mem,
    num::NonZero,
    ops::ControlFlow,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use termion::{
    cursor,
    event::Key,
    input::TermRead,
    raw::{IntoRawMode, RawTerminal},
    screen::{AlternateScreen, IntoAlternateScreen},
};
use tracing::{debug, error};

/// Width of the screen in terminal columns
const TERM_WIDTH: u16 = 60;
/// Height of the screen in terminal rows
const TERM_HEIGHT: u16 = 20;
/// Terminal escape code to trigger graphics rendering
///
/// https://sw.kovidgoyal.net/kitty/graphics-protocol/#the-graphics-escape-code
const ESCAPE: &str = "\u{1b}";
/// POSIX signals that tell the process to shut down
const QUIT_SIGNALS: [i32; 4] = [
    signal::SIGINT,
    signal::SIGHUP,
    signal::SIGQUIT,
    signal::SIGTERM,
];

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
    terminal: Terminal<TermionBackend<AlternateScreen<RawTerminal<Stdout>>>>,
    /// Flag set by the signal handler when a termination signal is received
    quit: Arc<AtomicBool>,
    /// Last time debug info was drawn to the screen
    ///
    /// `None` iff debug info has never been drawn. Used to throttle debug draw
    /// rate while running the emulator.
    last_debug_draw: Option<Instant>,
}

impl TerminalBackend {
    /// Initialize a new terminal adapter with the given pixel dimensions
    ///
    /// This will register listeners to listen for quit signals from the
    /// OS and spawn a background thread to listen for keyboard input.
    pub fn new() -> io::Result<Self> {
        let mut out = io::stdout().into_raw_mode()?.into_alternate_screen()?;
        write!(out, "{}", cursor::Hide)?;
        out.flush()?;
        let terminal = Terminal::new(TermionBackend::new(out))?;

        // Start a signal listener for SIGINT and friends.
        // We need to catch signals to allow the screen to clean up before exit.
        let quit = Arc::new(AtomicBool::new(false));
        for signal in QUIT_SIGNALS {
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
            terminal,
            quit,
            last_debug_draw: None,
        })
    }

    /// Run the emulator with the surrounding TUI
    ///
    /// If a [Debugger] is provided, run in debug mode and start paused.
    pub fn run(
        &mut self,
        emulator: &mut GameBoy,
        mut debugger: Option<Debugger>,
    ) {
        self.draw(emulator.frame());

        // Draw initial debug state
        if let Some(debugger) = &debugger {
            self.draw_debug(emulator, debugger);
        }

        // This loop runs constantly, even if the debugger is paused. Its run
        // rate is throttled by two things:
        // - End-of-frame sleep in the emulator (while unpaused)
        // - Input read timeout (while paused)
        while !self.quit.load(Ordering::Relaxed) {
            if let Some(debugger) = &mut debugger {
                if !debugger.paused() {
                    // After the tick, check breakpoints for pauses. Breakpoints
                    // only depend on emulator state, so we only need to check
                    // them after the state has changed.
                    emulator.tick(self);
                    debugger.check_breakpoints(emulator);
                }
                self.draw_debug(emulator, debugger);
            } else {
                // No debugger - just run normally
                emulator.tick(self);
            }

            // Check the input queue
            if self.drain_input(emulator, debugger.as_mut()).is_break() {
                break;
            }
        }
    }

    /// Drain all events from the input queue
    ///
    /// Return [ControlFlow::Break] if the loop should exit. A [Debugger] is
    /// provided only if running in debug mode. If not given, all debug events
    /// will be ignored.
    fn drain_input(
        &mut self,
        emulator: &GameBoy,
        mut debugger: Option<&mut Debugger>,
    ) -> ControlFlow<()> {
        // While the debugger is paused, we have nothing to do but wait for
        // input. In that case, we'll use a timeout on the input queue so we
        // don't burn a lot of CPU. It still needs to be short though so we can
        // still periodically check the `quit` flag.
        let input_timeout = if debugger.as_ref().is_some_and(|dbg| dbg.paused())
        {
            Duration::from_millis(100)
        } else {
            Duration::ZERO
        };

        // Drain the input queue
        while let Some(event) = self.next_event(input_timeout) {
            debug!(?event, "Input event");
            match event {
                // Primary events
                InputEvent::Quit => return ControlFlow::Break(()), // Exit
                InputEvent::Button(_) => todo!("TODO track input state"),

                // Debug events
                InputEvent::Debug(event)
                    if let Some(debugger) = &mut debugger =>
                {
                    match event {
                        DebugEvent::PauseToggle => debugger.toggle_pause(),
                        DebugEvent::StepCycle => debugger.step_cycle(emulator),
                        DebugEvent::StepFrame => debugger.step_frame(emulator),
                        DebugEvent::StepInstruction => {
                            debugger.step_instruction(emulator);
                        }
                    }
                }
                InputEvent::Debug(_) => {}
            }
        }
        ControlFlow::Continue(())
    }

    /// Load the next input event from the event queue
    fn next_event(&mut self, timeout: Duration) -> Option<InputEvent> {
        // Grab the next event off the queue
        match self.input_rx.recv_timeout(timeout) {
            Ok(event) => Some(event),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => todo!(),
        }
    }

    /// Draw debug info to the screen
    ///
    /// This call is automatically throttled to a maximum framerate. While the
    /// emulator is running, there isn't value in drawing on every single tick
    /// and it causes a huge slowdown.
    fn draw_debug(&mut self, emulator: &GameBoy, debugger: &Debugger) {
        const MIN_DRAW_GAP: Duration = Duration::from_millis(100);

        let now = Instant::now();
        let should_draw = debugger.paused()
            || self
                .last_debug_draw
                .is_none_or(|instant| now - instant >= MIN_DRAW_GAP);
        if should_draw {
            // Debug UI is drawn via ratatui
            let result = self.terminal.draw(|frame| {
                frame.render_widget(
                    DebugInfo { emulator, debugger },
                    frame.area(),
                );
            });
            if let Err(error) = result {
                error!("Error drawing to terminal: {error}");
            }
            self.last_debug_draw = Some(now);
        }
    }

    /// Monitor stdin for input
    ///
    /// When a relevant event is received, it's pushed into the given channel.
    /// This will only terminate on an error or end of stream, so it should run
    /// in a background thread.
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
            Key::Char(' ') => Some(InputEvent::Debug(DebugEvent::PauseToggle)),
            Key::Right => Some(InputEvent::Debug(DebugEvent::StepInstruction)),
            Key::CtrlRight => Some(InputEvent::Debug(DebugEvent::StepCycle)),
            Key::ShiftRight => Some(InputEvent::Debug(DebugEvent::StepFrame)),
            Key::Char('q') | Key::Ctrl('c') => Some(InputEvent::Quit),
            _ => None,
        }
    }
}

impl Backend for TerminalBackend {
    fn draw(&mut self, frame: &FrameBuffer) {
        // This rendering does *not* use ratatui, because we need to write
        // directly to the output
        let mut f = || {
            // Shitty try block
            write!(self.terminal.backend_mut(), "{}", cursor::Goto(1, 1))?;
            draw_frame(frame, false, self.terminal.backend_mut())
        };
        if let Err(error) = f() {
            error!("Error drawing to terminal: {error}");
        }
    }
}

/// Widget to draw debug info to the terminal
struct DebugInfo<'a> {
    emulator: &'a GameBoy,
    debugger: &'a Debugger,
}

impl Widget for DebugInfo<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let [left_area, memory_area] =
            Layout::horizontal([TERM_WIDTH.into(), Constraint::Min(0)])
                .areas(area);
        // Leave space for the screen in the top-left
        let [_, mut bottom_left_area] =
            Layout::vertical([TERM_HEIGHT.into(), Constraint::Min(0)])
                .areas(left_area);
        bottom_left_area.width += 1; // Combine borders into the Memory panel
        // Move down below the screen area
        let [basic_area, cpu_area] =
            Layout::horizontal([Constraint::Min(0), 36.into()])
                .spacing(-1)
                .areas(bottom_left_area);

        let basic_area = panel("Basic", basic_area, buf);
        let paused = if self.debugger.paused() {
            "PAUSED"
        } else {
            "RUNNING"
        };

        // Basic
        Text::from_iter([
            format!("CLOCK: {}", self.emulator.clock().cycles()),
            format!("DBG: {paused}"),
        ])
        .render(basic_area, buf);

        // CPU
        self.emulator.cpu().render(cpu_area, buf);

        // Memory
        panel("Memory", memory_area, buf);
    }
}

impl Drop for TerminalBackend {
    fn drop(&mut self) {
        let _ = write!(self.terminal.backend_mut(), "{}", cursor::Show);
    }
}

impl Widget for &Cpu {
    fn render(self, area: Rect, buf: &mut Buffer) {
        /// Register display helper
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

        let area = panel("CPU", area, buf);

        let previous = self.previous_instruction().unwrap_or(InstructionInfo {
            instruction: Instruction::Invalid,
            duration: Cycles(0),
            end: Cycles(0),
            size: 0,
        });
        let next = self.current_instruction();
        let registers = self.registers();
        let lines = [
            format!(
                "PREV: {instr} ({dur}cy/{size}B)",
                instr = previous.instruction,
                dur = previous.duration,
                size = previous.size,
            ),
            format!(
                "NEXT: {instr} ({dur}cy/{size}B)",
                instr = next.instruction,
                dur = next.duration,
                size = next.size,
            ),
            // Registers
            format!("pc: {}", registers.pc()),
            format!("sp: {}", registers.sp()),
            format!("a: {}", Reg(registers.a())),
            format!(
                "f: {} {}",
                IntDisplay::hex(registers.f().as_u8()),
                registers.f().unpack()
            ),
            format!("af: {}", Reg(registers.af())),
            format!("b: {}", Reg(registers.b())),
            format!("c: {}", Reg(registers.c())),
            format!("bc: {}", Reg(registers.bc())),
            format!("d: {}", Reg(registers.d())),
            format!("e: {}", Reg(registers.e())),
            format!("de: {}", Reg(registers.de())),
            format!("h: {}", Reg(registers.h())),
            format!("l: {}", Reg(registers.l())),
            format!("hl: {}", Reg(registers.hl())),
            format!(
                "INT: {}",
                if self.interrupts_enabled() {
                    "ENABLE"
                } else {
                    "DISABLE"
                }
            ),
        ];
        Text::from_iter(lines).render(area, buf);
    }
}

/// Draw an outline for a panel, returning the inner area
fn panel(title: &'_ str, area: Rect, buf: &mut Buffer) -> Rect {
    let block = Block::new()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .merge_borders(MergeStrategy::Fuzzy);
    (&block).render(area, buf);
    block.inner(area)
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

/// A processed input event
///
/// This is a mapped input event to its app-relevant meaning.
#[derive(Debug)]
enum InputEvent {
    /// Input mapped to an emulated button on the Game Boy
    #[expect(unused)]
    Button(Button),
    /// Input specific to the debugger
    ///
    /// This is a sub-enum so it can be easily ignored when debug is disabled.
    Debug(DebugEvent),
    /// Exit the app
    Quit,
}

/// An input event specific to the debugger
#[derive(Debug)]
enum DebugEvent {
    /// Pause or unpause execution in the debugger
    PauseToggle,
    /// Advance the debugger one clock cycle
    StepCycle,
    /// Advance the debugger to the end of the current frame
    StepFrame,
    /// Advance the debugger to the end of the current CPU instruction
    StepInstruction,
}

/// Pressable Buttons on a Game Boy
#[derive(Debug)]
#[expect(unused)]
enum Button {
    A,
    B,
    Up,
    Down,
    Left,
    Right,
    Start,
    Select,
}
