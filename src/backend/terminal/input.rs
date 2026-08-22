//! Terminal input handling

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// A processed input event
///
/// This is a mapped input event to its app-relevant meaning.
#[derive(Debug)]
pub enum InputEvent {
    /// Exit the app
    Quit,
    /// An event that should be routed to the TUI
    Tui(TuiEvent),
}

impl From<TuiEvent> for InputEvent {
    fn from(v: TuiEvent) -> Self {
        Self::Tui(v)
    }
}

/// An input event intended for the TUI
#[derive(Debug)]
pub enum TuiEvent {
    /// Navigate up
    Up,
    /// Navigate down
    Down,
    /// Navigate right, wait no I mean left, wait, yeah fuck uhhh it's left
    Left,
    /// Navigate right
    Right,
    /// Pause or unpause execution in the debugger
    DebugPauseToggle,
    /// Advance the debugger one clock cycle
    DebugStepCycle,
    /// Advance the debugger to the end of the current frame
    DebugStepFrame,
    /// Advance the debugger to the end of the current CPU instruction
    DebugStepInstruction,
}

/// Map a crossterm event to an [InputEvent]
pub fn map_event(event: Event) -> Option<InputEvent> {
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
        KeyCode::Char('h') => TuiEvent::Left.into(),
        KeyCode::Char('j') => TuiEvent::Down.into(),
        KeyCode::Char('k') => TuiEvent::Up.into(),
        KeyCode::Char('l') => TuiEvent::Right.into(),
        KeyCode::Char(' ') => TuiEvent::DebugPauseToggle.into(),
        KeyCode::Right if modi(KeyModifiers::CONTROL) => {
            TuiEvent::DebugStepCycle.into()
        }
        KeyCode::Right if modi(KeyModifiers::SHIFT) => {
            TuiEvent::DebugStepFrame.into()
        }
        KeyCode::Right => TuiEvent::DebugStepInstruction.into(),
        KeyCode::Char('q') => InputEvent::Quit,
        KeyCode::Char('c') if modi(KeyModifiers::CONTROL) => InputEvent::Quit,
        _ => return None,
    };
    Some(event)
}
