//! Hardware bindings for the terminal

mod draw;
mod input;
mod tui;

pub use draw::draw_frame;

use crate::{
    Debugger,
    backend::{
        Backend, FrameBuffer,
        terminal::{
            input::{DebugEvent, InputEvent},
            tui::DebugInfo,
        },
    },
    emu::GameBoy,
};
use crossterm::{
    cursor,
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
    },
    time::{Duration, Instant},
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

/// A [Backend] implementation to draw to the terminal
///
/// This uses the [Kitty Terminal Graphics Protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/)
/// to draw to stdout. It reads input from stdin.
pub struct TerminalBackend {
    /// Channel to write output to (stdout)
    terminal: Terminal<CrosstermBackend<Stdout>>,
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
        initialize_panic_handler();
        initialize_terminal()?;

        let terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;

        // Start a signal listener for SIGINT and friends.
        // We need to catch signals to allow the screen to clean up before exit.
        let quit = Arc::new(AtomicBool::new(false));
        for signal in QUIT_SIGNALS {
            signal_hook::flag::register(signal, quit.clone()).unwrap();
        }

        Ok(Self {
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
        while let Some(event) = input::next_event(input_timeout) {
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
