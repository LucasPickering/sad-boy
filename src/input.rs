use signal_hook::consts::signal;
use std::{
    io::{self, Stdin},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use termion::{
    event::Key,
    input::{Keys, TermRead},
};
use tracing::error;

/// An interface for user input
///
/// This is an abstraction over input hardware. The backend could be a terminal,
/// web browser, etc.
pub trait Input {
    /// TODO
    fn next(&mut self) -> Option<InputEvent>;

    /// Should the emulator exit?
    ///
    /// This is called by the emulator on each tick and should be used to
    /// monitor external exit conditions (such as process signals).
    ///
    /// This should *not* check for a [InputEvent::Quit] input. That will be
    /// monitored separately via [Self::next].
    fn should_quit(&self) -> bool;
}

/// TODO
pub enum InputEvent {
    /// Exit the app
    Quit,
    /// Advance the debugger one step
    StepNext,
}

/// An [Input] implementation to read events from the terminal
///
/// This will read input from stdin. It also runs a background thread to listen
/// for termination signals and exits when one is received.
pub struct TerminalInput {
    /// Channel for reading user input
    keys: Keys<Stdin>,
    /// Flag set by the signal handler when a termination signal is received
    quit: Arc<AtomicBool>,
}

impl TerminalInput {
    pub fn new() -> Self {
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

        Self {
            keys: io::stdin().keys(),
            quit,
        }
    }
}

impl Input for TerminalInput {
    fn next(&mut self) -> Option<InputEvent> {
        match self.keys.next() {
            Some(Ok(key)) => match key {
                Key::Char('q') => Some(InputEvent::Quit),
                Key::Right => Some(InputEvent::StepNext),
                _ => None,
            },
            Some(Err(error)) => {
                error!("Error reading input: {error}");
                None
            }
            None => None,
        }
    }

    fn should_quit(&self) -> bool {
        self.quit.load(Ordering::Relaxed)
    }
}

/// TODO
pub struct HeadlessInput {
    should_quit: Box<dyn Fn() -> bool>,
}

impl HeadlessInput {
    pub fn new(should_quit: impl 'static + Fn() -> bool) -> Self {
        Self {
            should_quit: Box::new(should_quit),
        }
    }
}

impl Input for HeadlessInput {
    fn next(&mut self) -> Option<InputEvent> {
        None
    }

    fn should_quit(&self) -> bool {
        (self.should_quit)()
    }
}
