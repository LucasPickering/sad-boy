//! Hardware bindings for the terminal

mod draw;
mod input;
mod tui;

pub use draw::draw_frame;

use crate::{
    Debugger,
    backend::{
        Backend, FrameBuffer,
        terminal::{input::InputEvent, tui::Tui},
    },
    emu::GameBoy,
};
use crossterm::{
    cursor, event,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{Terminal, prelude::CrosstermBackend};
use signal_hook::consts::signal;
use std::{
    io::{self, Stdout, Write},
    ops::ControlFlow,
    panic,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};
use tracing::{debug, error, info};

/// Width of the screen in terminal columns
const TERM_WIDTH: u16 = 60;
/// Height of the screen in terminal rows
const TERM_HEIGHT: u16 = 20;
/// POSIX signals that tell the process to shut down
const QUIT_SIGNALS: [i32; 4] = [
    signal::SIGINT,
    signal::SIGHUP,
    signal::SIGQUIT,
    signal::SIGTERM,
];

type RatatuiTerminal = Terminal<CrosstermBackend<Stdout>>;

/// A [Backend] implementation to draw to the terminal
///
/// This uses the [Kitty Terminal Graphics Protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/)
/// to draw to stdout. It reads input from stdin.
pub struct TerminalBackend {
    /// Receiver channel for input events
    input: mpsc::Receiver<InputEvent>,
    /// Flag set by the signal handler when a termination signal is received
    quit: Arc<AtomicBool>,
    /// Channel to write output to (stdout)
    terminal: RatatuiTerminal,
    /// Interactive portions around the emulator screen
    tui: Tui,
}

impl TerminalBackend {
    /// Initialize a new terminal adapter with the given pixel dimensions
    ///
    /// This will register listeners to listen for quit signals from the
    /// OS and spawn a background thread for the input queue.
    pub fn new() -> io::Result<Self> {
        initialize_panic_handler();
        initialize_terminal()?;

        let terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;

        // Start a signal listener for SIGINT and friends.
        // We need to catch signals to allow the screen to clean up before exit.
        let quit = Arc::new(AtomicBool::new(false));
        for signal in QUIT_SIGNALS {
            signal_hook::flag::register(signal, quit.clone()).unwrap();
        }

        // Input listens in a background thread because it's much faster
        let (input_tx, input_rx) = mpsc::channel();
        thread::Builder::new()
            .name("input".to_owned())
            .spawn(move || Self::input_thread(input_tx))?;

        Ok(Self {
            input: input_rx,
            quit,
            terminal,
            tui: Tui::default(),
        })
    }

    /// Run the emulator with the surrounding TUI
    ///
    /// If a [Debugger] is provided, run in debug mode and start paused.
    pub fn run(&mut self, emulator: &mut GameBoy, mut debugger: Debugger) {
        self.draw(emulator.frame());

        // Draw initial TUI
        self.tui.draw(&mut self.terminal, emulator, &debugger);

        // This loop runs constantly, even if the debugger is paused. Its run
        // rate is throttled by two things:
        // - End-of-frame sleep in the emulator (while unpaused)
        // - Input read timeout (while paused)
        while !self.quit.load(Ordering::Relaxed) {
            if debugger.run_state().should_tick() {
                // After the tick, check breakpoints for pauses. Breakpoints
                // only depend on emulator state, so we only need to check
                // them after the state has changed.
                emulator.tick(self);
                debugger.check_breakpoints(emulator);
            }

            // Check for input
            let handled_event =
                match self.handle_next_event(emulator, &mut debugger) {
                    ControlFlow::Break(()) => break,
                    ControlFlow::Continue(handled) => handled,
                };
            // Only redraw the TUI if there was at least one input event.
            // Without input, the screen can't change. Drawing on every tick is
            // extraordinarily expensive.
            if handled_event || debugger.paused() {
                self.tui.draw(&mut self.terminal, emulator, &debugger);
            }
        }
    }

    /// Pop the next event off the input queue and handle it
    ///
    /// ## Return
    ///
    /// - `ControlFlow::Continue(true)` if at least one event was handled
    /// - `ControlFlow::Continue(false)` if the queue was empty
    /// - `ControlFlow::Break` if the loop should exit (quit event)
    fn handle_next_event(
        &mut self,
        emulator: &GameBoy,
        debugger: &mut Debugger,
    ) -> ControlFlow<(), bool> {
        // While the debugger is paused, we have nothing to do but wait for
        // input. In that case, we'll use a timeout on the input queue so we
        // don't burn a lot of CPU. It still needs to be short though so we can
        // periodically check the `quit` flag.
        let input_timeout = if debugger.run_state().should_tick() {
            Duration::ZERO
        } else {
            Duration::from_millis(100)
        };

        // Drain the input queue
        if let Some(event) = self.next_event(input_timeout) {
            debug!(?event, "Input event");
            match event {
                // Primary events
                InputEvent::Quit => return ControlFlow::Break(()),
                InputEvent::Tui(event) => {
                    self.tui.update(emulator, debugger, event);
                }
            }
            ControlFlow::Continue(true)
        } else {
            ControlFlow::Continue(false)
        }
    }

    /// Load the next input event from the queue
    ///
    /// Return `None` if there was an error or no event occurred within the
    /// given timeout.
    fn next_event(&self, timeout: Duration) -> Option<InputEvent> {
        match self.input.recv_timeout(timeout) {
            Ok(event) => Some(event),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            // Input thread is never supposed to exit
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                panic!("Input channel closed")
            }
        }
    }

    /// Background thread monitoring input
    ///
    /// Polling the input in the main thread is too slow, so it's pushed into a
    /// background thread. The main thread just has to check an mpsc channel,
    /// which is much faster.
    fn input_thread(tx: mpsc::Sender<InputEvent>) {
        loop {
            // I'm using unwrap() because an error indicates a bug
            let event = event::read().unwrap();
            if let Some(event) = input::map_event(event) {
                tx.send(event).unwrap();
            }
        }
    }
}

impl Drop for TerminalBackend {
    fn drop(&mut self) {
        let _ = restore_terminal();
    }
}

impl Backend for TerminalBackend {
    fn draw(&mut self, frame: &FrameBuffer) {
        // This rendering does *not* use ratatui, because we need to write
        // directly to the output
        let mut f = || {
            // Shitty try block
            write!(self.terminal.backend_mut(), "{}", cursor::MoveTo(0, 0))?;
            draw_frame(frame, false, self.terminal.backend_mut())
        };
        if let Err(error) = f() {
            error!("Error drawing to terminal: {error}");
        }
    }
}

/// Set up terminal for the TUI
fn initialize_terminal() -> io::Result<()> {
    info!("Initializing terminal");
    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(io::stdout(), EnterAlternateScreen)?;
    Ok(())
}

/// Return terminal to initial state
fn restore_terminal() -> io::Result<()> {
    info!("Restoring terminal");
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(io::stdout(), LeaveAlternateScreen)?;
    Ok(())
}

/// Restore terminal state during a panic
fn initialize_panic_handler() {
    let original_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        let _ = restore_terminal();
        original_hook(panic_info);
    }));
}
