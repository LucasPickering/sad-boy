/// A processed input event
///
/// This is a mapped input event to its app-relevant meaning.
pub enum InputEvent {
    /// Exit the app
    Quit,
    /// Advance the debugger one step
    StepNext,
}
