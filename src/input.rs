/// A processed input event
///
/// This is a mapped input event to its app-relevant meaning.
pub enum InputEvent {
    /// Input mapped to an emulated button on the Game Boy
    #[expect(unused)]
    Button(Button),
    /// Pause or unpause execution in the debugger
    DebugPauseToggle,
    /// Advance the debugger one step
    DebugStepNext,
    /// Exit the app
    Quit,
}

/// Pressable Buttons on a Game Boy
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
