//! Terminal input handling

use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};
use std::time::Duration;

/// A processed input event
///
/// This is a mapped input event to its app-relevant meaning.
#[derive(Debug)]
pub enum InputEvent {
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
pub enum DebugEvent {
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
pub enum Button {
    A,
    B,
    Up,
    Down,
    Left,
    Right,
    Start,
    Select,
}

/// Load the next input event from the terminal
///
/// Return `None` if there was an error or no event occurred within the
/// given timeout.
pub fn next_event(timeout: Duration) -> Option<InputEvent> {
    // It's possible that polling the terminal directly in the main loop
    // will be too slow. In that case, we can punt this to another thread.
    if event::poll(timeout).unwrap() {
        let event = event::read().unwrap();
        map_event(event)
    } else {
        None
    }
}

/// Map a crossterm event to an [InputEvent]
fn map_event(event: Event) -> Option<InputEvent> {
    let Event::Key(KeyEvent {
        kind: KeyEventKind::Press,
        code,
        modifiers,
        ..
    }) = event
    else {
        return None;
    };
    let modi = |modifier| modifiers.contains(modifier);
    // TODO use a better dynamic mapping (steal from slumber)
    let event = match code {
        KeyCode::Char(' ') => InputEvent::Debug(DebugEvent::PauseToggle),
        KeyCode::Right if modi(KeyModifiers::CONTROL) => {
            InputEvent::Debug(DebugEvent::StepCycle)
        }
        KeyCode::Right if modi(KeyModifiers::SHIFT) => {
            InputEvent::Debug(DebugEvent::StepFrame)
        }
        KeyCode::Right => InputEvent::Debug(DebugEvent::StepInstruction),
        KeyCode::Char('q') => InputEvent::Quit,
        KeyCode::Char('c') if modi(KeyModifiers::CONTROL) => InputEvent::Quit,
        _ => return None,
    };
    Some(event)
}
