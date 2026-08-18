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
    /// Pause or unpause execution in the debugger
    DebugPauseToggle,
    /// Advance the debugger one clock cycle
    DebugStepCycle,
    /// Advance the debugger to the end of the current frame
    DebugStepFrame,
    /// Advance the debugger to the end of the current CPU instruction
    DebugStepInstruction,
    /// Exit the app
    Quit,
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
        KeyCode::Char(' ') => InputEvent::DebugPauseToggle,
        KeyCode::Right if modi(KeyModifiers::CONTROL) => {
            InputEvent::DebugStepCycle
        }
        KeyCode::Right if modi(KeyModifiers::SHIFT) => {
            InputEvent::DebugStepFrame
        }
        KeyCode::Right => InputEvent::DebugStepInstruction,
        KeyCode::Char('q') => InputEvent::Quit,
        KeyCode::Char('c') if modi(KeyModifiers::CONTROL) => InputEvent::Quit,
        _ => return None,
    };
    Some(event)
}
