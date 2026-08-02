//! Graphics bindings for the terminal

#[cfg(test)]
use crate::screen::Color;
use crate::{
    input::InputEvent,
    screen::{FrameBuffer, draw_frame},
};
use signal_hook::consts::signal;
use std::{
    io::{self, Stdin, Stdout, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use termion::{
    cursor,
    event::Key,
    input::{Keys, TermRead},
    screen::{AlternateScreen, IntoAlternateScreen},
};
use tracing::error;

/// An interface for a screen and input
///
/// This is an abstraction over hardware. The backend could be a terminal, web
/// browser, etc.
pub trait Backend {
    /// Draw the given frame buffer to the terminal
    fn draw(&mut self, frame: &FrameBuffer);

    /// TODO
    fn next_event(&mut self) -> Option<InputEvent>;

    /// Should the emulator exit?
    ///
    /// This is called by the emulator on each tick and should be used to
    /// monitor external exit conditions (such as process signals).
    ///
    /// This should *not* check for a [InputEvent::Quit] input. That will be
    /// monitored separately via [Self::next].
    fn should_quit(&self) -> bool;
}

/// A [Backend] implementation to draw to the terminal
///
/// This uses the [Kitty Terminal Graphics Protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/)
/// to draw to stdout. It reads input from stdin.
pub struct TerminalBackend {
    /// Channel for reading user input
    keys: Keys<Stdin>,
    /// Channel to write output to (stdout)
    out: AlternateScreen<Stdout>,
    /// Flag set by the signal handler when a termination signal is received
    quit: Arc<AtomicBool>,
}

impl TerminalBackend {
    /// Initialize a new screen adapter with the given pixel dimensions
    pub fn new() -> io::Result<Self> {
        let mut out = io::stdout().into_alternate_screen()?;
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

        Ok(Self {
            keys: io::stdin().keys(),
            out,
            quit,
        })
    }
}

impl Backend for TerminalBackend {
    fn draw(&mut self, frame: &FrameBuffer) {
        if let Err(error) = draw_frame(frame, false, &mut self.out) {
            error!("Error drawing to screen: {error}");
        }
    }

    fn next_event(&mut self) -> Option<InputEvent> {
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

/// An in-memory [Backend] for testing and headless operation
pub struct HeadlessBackend {
    /// Most recent drawn frame
    last_frame: Option<FrameBuffer>,
    /// Callback to check if the app should quit
    ///
    /// Tests use this to define custom termination conditions.
    should_quit: Box<dyn Fn() -> bool>,
}

impl HeadlessBackend {
    pub fn new(should_quit: impl 'static + Fn() -> bool) -> Self {
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
        assert_eq!(
            expected.len(),
            frame.pixels.len(),
            "Expected pixel array must be length {} * {}",
            frame.width,
            frame.height
        );

        let mut mismatched: Vec<(u16, u16, Color, Color)> = vec![];
        for (i, (color_actual, color_expected)) in
            frame.pixels.iter().zip(expected).enumerate()
        {
            if color_actual != color_expected {
                let i = i as u16;
                let x = i % frame.width;
                let y = i / frame.width;
                mismatched.push((x, y, *color_actual, *color_expected));
            }
        }

        if !mismatched.is_empty() {
            // Print the screens
            // TODO the expected overwites the actual right now
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
    fn draw(&mut self, frame: &FrameBuffer) {
        self.last_frame = Some(frame.clone());
    }

    fn next_event(&mut self) -> Option<InputEvent> {
        None
    }

    fn should_quit(&self) -> bool {
        (self.should_quit)()
    }
}
