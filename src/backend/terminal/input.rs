//! Terminal input handling

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// An event triggered by user input
///
/// This includes the original [KeyEvent] as well as the mapped [InputAction]
/// (if any). Most consumers just want the `action`, but widgets that process
/// all events (such as text inputs) will want the raw event.
#[derive(Debug)]
pub struct InputEvent {
    /// Raw input event
    pub event: KeyEvent,
    /// Bound action
    pub action: Option<InputAction>,
}

impl InputEvent {
    /// Map a raw input event to [InputEvent]
    ///
    ///
    ///
    /// If the event is bound to a particular action, [Self::action] will be
    /// populated. Return `None` if the event should be ignored (e.g. mouse
    /// events).
    pub fn from_event(event: Event) -> Option<Self> {
        if let Event::Key(
            event @ KeyEvent {
                kind: KeyEventKind::Press,
                ..
            },
        ) = event
        {
            Some(Self {
                event,
                action: InputAction::from_event(event),
            })
        } else {
            None
        }
    }
}

/// A semantic action triggered by inpu
#[derive(Debug)]
pub enum InputAction {
    /// Exit the app; this cannot be consumed by anyone else
    ForceQuit,
    /// An event that should be routed to the TUI
    Tui(TuiAction),
    /// Game Boy button that should be routed to the emulator
    #[expect(unused)]
    Button(Button),
}

impl InputAction {
    /// Map a crossterm event to an [InputAction]
    ///
    /// Return `None` if the input is unbound.
    fn from_event(event: KeyEvent) -> Option<Self> {
        let KeyEvent {
            code, modifiers, ..
        } = event;
        let ctrl = modifiers.contains(KeyModifiers::CONTROL);
        let shift = modifiers.contains(KeyModifiers::SHIFT);
        let event = match code {
            KeyCode::Char('h') => TuiAction::Left.into(),
            KeyCode::Char('j') => TuiAction::Down.into(),
            KeyCode::Char('k') => TuiAction::Up.into(),
            KeyCode::Char('l') => TuiAction::Right.into(),
            KeyCode::Esc => TuiAction::Cancel.into(),
            KeyCode::Enter => TuiAction::Submit.into(),
            KeyCode::Char('g') => TuiAction::DebugGoToAddress.into(),
            KeyCode::Char(' ') => TuiAction::DebugPauseToggle.into(),
            KeyCode::Char('c') if ctrl => InputAction::ForceQuit,
            KeyCode::Right if ctrl => TuiAction::DebugStepCycle.into(),
            KeyCode::Right if shift => TuiAction::DebugStepFrame.into(),
            KeyCode::Right => TuiAction::DebugStepInstruction.into(),
            KeyCode::Left => TuiAction::DebugStepBack.into(),
            _ => return None,
        };
        Some(event)
    }
}

impl From<Button> for InputAction {
    fn from(value: Button) -> Self {
        Self::Button(value)
    }
}

impl From<TuiAction> for InputAction {
    fn from(v: TuiAction) -> Self {
        Self::Tui(v)
    }
}

/// A mapped input action intended for the TUI
#[derive(Debug)]
pub enum TuiAction {
    /// Navigate up
    Up,
    /// Navigate down
    Down,
    /// Navigate right, wait no I mean left, wait, yeah fuck uhhh it's left
    Left,
    /// Navigate right
    Right,
    /// Cancel the current action
    Cancel,
    /// Submit an entry (e.g. from a text box)
    Submit,
    /// Go to a memory address in the memory panel
    DebugGoToAddress,
    /// Pause or unpause execution in the debugger
    DebugPauseToggle,
    /// Step back to a previous emulator snapshot
    DebugStepBack,
    /// Advance the debugger one clock cycle
    DebugStepCycle,
    /// Advance the debugger to the end of the current frame
    DebugStepFrame,
    /// Advance the debugger to the end of the current CPU instruction
    DebugStepInstruction,
}

/// Game Boy buttons
#[derive(Debug)]
#[expect(unused)]
pub enum Button {
    A,
    B,
    Start,
    Select,
    Up,
    Down,
    Left,
    Right,
}
