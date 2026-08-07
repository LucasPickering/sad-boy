/// A processed input event
///
/// This is a mapped input event to its app-relevant meaning.
pub enum InputEvent {
    /// Pause or unpause execution in the debugger
    DebugPauseToggle,
    /// Advance the debugger one step
    DebugStepNext,
    /// Exit the app
    Quit,
}
