//! Graphics bindings for the terminal

#[cfg(test)]
use crate::screen::Color;
use crate::{
    emu::{Cycles, DebugInfo, Instruction},
    input::InputEvent,
    screen::{FrameBuffer, draw_frame},
    util::IntDisplay,
};
use signal_hook::consts::signal;
use std::{
    fmt::{self, Display},
    io::{self, Stdout, Write},
    panic,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
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
    /// Initialize a new screen adapter with the given pixel dimensions
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
        // TODO make start height dynamic
        for (line, y) in lines.iter().zip(1..) {
            // Terminal is in raw mode so we have to move the cursor and clear
            // the line manually
            write!(
                self.out,
                "{goto}{clear}{line}",
                goto = cursor::Goto(1, y),
                clear = clear::CurrentLine,
            )?;
        }
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
        let (prev_instruction, prev_cycles) = cpu
            .previous_instruction
            .unwrap_or((Instruction::Invalid, Cycles(0)));
        let (next_instruction, next_cycles) = cpu.next_instruction;
        let result = self.write_debug(&[
            format_args!("Clock: {}", info.clock_cycles),
            format_args!("=== CPU ==="),
            format_args!("Prev: {prev_instruction} ({prev_cycles} cycles)"),
            format_args!("Next: {next_instruction} ({next_cycles} cycles)"),
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
    pub fn assert_pixels(&self, expected: &[Color]) {
        use std::fmt::Write;

        let frame = self
            .last_frame
            .as_ref()
            .expect("Screen has not been drawn to");
        let actual = frame.pixels();
        assert_eq!(
            actual.len(),
            expected.len(),
            "Expected pixel array must be length {} * {}",
            frame.width(),
            frame.height()
        );

        let mut mismatched: Vec<(u16, u16, Color, Color)> = vec![];
        for (i, (color_actual, color_expected)) in
            actual.iter().zip(expected).enumerate()
        {
            if color_actual != color_expected {
                let i = i as u16;
                let x = i % frame.width();
                let y = i / frame.width();
                mismatched.push((x, y, *color_actual, *color_expected));
            }
        }

        if !mismatched.is_empty() {
            // Print the screens
            // TODO the expected overwrites the actual right now
            self.draw_pixels("Actual", frame);
            // self.draw_pixels("Expected", expected);

            // Show mismatched cells, but cap it to prevent absurd amounts of
            // output
            let mut messages = String::new();
            let truncated = mismatched.get(0..10).unwrap_or(&mismatched);
            for (x, y, actual, expected) in truncated {
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
        let mut stderr = io::stderr();
        writeln!(stderr, "{title}:").unwrap();
        draw_frame(frame, true, &mut stderr).unwrap();
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
