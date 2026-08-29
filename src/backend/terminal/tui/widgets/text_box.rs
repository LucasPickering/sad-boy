use crate::backend::terminal::tui::style::STYLES;
use crossterm::{
    cursor,
    event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
};
use ratatui::{
    prelude::{Buffer, Rect},
    text::{Line, Text},
    widgets::{Paragraph, StatefulWidget, Widget},
};
use std::{cell::Cell, io, marker::PhantomData, str::FromStr};

/// A single-line text box widget
pub struct TextBox<T = String> {
    /// Current text
    state: TextState,
    /// Text to show when text content is empty
    placeholder_text: String,
    _phantom: PhantomData<T>,
}

impl<T: FromStr> TextBox<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fluent setter to set the text that will render when the input is empty
    pub fn with_placeholder_text(
        mut self,
        placeholder_text: impl Into<String>,
    ) -> Self {
        self.placeholder_text = placeholder_text.into();
        self
    }

    /// TODO
    pub fn value(&self) -> Result<T, T::Err> {
        self.state.text.parse::<T>()
    }

    /// TODO
    pub fn clear(&mut self) {
        self.state = TextState::default();
    }

    /// TODO
    pub fn input(&mut self, event: KeyEvent) -> bool {
        let KeyEvent {
            code,
            modifiers,
            kind,
            ..
        } = event;
        match kind {
            KeyEventKind::Release => return false,
            KeyEventKind::Press | KeyEventKind::Repeat => {}
        }

        match code {
            KeyCode::Char(c) => self.state.insert(c),
            KeyCode::Backspace => self.state.delete_left(),
            KeyCode::Delete => self.state.delete_right(),
            KeyCode::Left => {
                if modifiers == KeyModifiers::CONTROL {
                    self.state.home();
                } else {
                    self.state.left();
                }
            }
            KeyCode::Right => {
                if modifiers == KeyModifiers::CONTROL {
                    self.state.end();
                } else {
                    self.state.right();
                }
            }
            KeyCode::Home => self.state.home(),
            KeyCode::End => self.state.end(),
            _ => return false, // Event unhandled
        }
        true
    }

    fn is_valid(&self) -> bool {
        self.state.text.parse::<T>().is_ok()
    }
}

impl<T> Default for TextBox<T> {
    fn default() -> Self {
        Self {
            state: Default::default(),
            placeholder_text: Default::default(),
            _phantom: Default::default(),
        }
    }
}

impl<T: FromStr> StatefulWidget for &TextBox<T> {
    /// Focused/unfocused
    ///
    /// It's a shortcut for now
    type State = bool;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let text: Text = if self.state.text.is_empty() {
            Line::from(self.placeholder_text.as_str())
                .style(STYLES.text_box_placeholder)
                .into()
        } else {
            self.state.text.as_str().into()
        };

        // Draw the text
        let text_stats = self.state.text_stats();
        let scroll_x = self.state.update_scroll(text_stats, area.width);
        let style = if self.is_valid() {
            STYLES.text_box_text
        } else {
            // Invalid and error state look the same
            STYLES.text_box_invalid
        };
        Paragraph::new(text)
            .scroll((0, scroll_x))
            .style(style)
            .render(area, buf);

        if *state {
            // Use the terminal's native cursor
            crossterm::execute!(
                io::stdout(),
                cursor::MoveTo(
                    area.x + text_stats.cursor_offset as u16 - scroll_x,
                    area.y
                ),
                cursor::Show,
            );
        }
    }
}

/// Encapsulation of text/cursor state. Encapsulating this makes reading and
/// testing the functionality easier.
#[derive(Debug, Default)]
#[cfg_attr(test, derive(PartialEq))]
struct TextState {
    text: String,
    /// **Byte** (not character) index in the text. Must be in the range `[0,
    /// text.len()]`. This must always fall on a character boundary.
    cursor: usize,
    /// Left/right scrolling, in _characters_. Scrolling can't be modified
    /// directly by the user. We shift left/right as needed to prevent the
    /// cursor from moving off screen. This is in a `Cell` because it needs
    /// to be modified during the draw phase, based on view width.
    scroll_x: Cell<u16>,
}

impl TextState {
    /// Is the cursor at the beginning of the text?
    fn is_at_home(&self) -> bool {
        self.cursor == 0
    }

    /// Is the cursor at the end of the text?
    fn is_at_end(&self) -> bool {
        self.cursor == self.char_len()
    }

    /// Get the number of **characters* (not bytes) in the text
    fn char_len(&self) -> usize {
        self.text.chars().count()
    }

    /// Move cursor to the beginning of text
    fn home(&mut self) {
        self.cursor = 0;
    }

    /// Move cursor to the end of text
    fn end(&mut self) {
        self.cursor = self.text.len();
    }

    /// Insert one character at the current cursor position
    fn insert(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    /// Move cursor left one **character**. This may be multiple bytes, if the
    /// character to the left is multiple bytes.
    fn left(&mut self) {
        if !self.is_at_home() {
            // unstable: use floor_char_boundary
            // https://github.com/rust-lang/rust/issues/93743
            // We know there's a char to the left, but we don't know how long
            // it is. Keep jumping left until we've hit a char boundary
            self.cursor -= 1;
            while !self.text.is_char_boundary(self.cursor) {
                self.cursor -= 1;
            }
        }
    }

    /// Move cursor right one character
    fn right(&mut self) {
        if !self.is_at_end() {
            // unstable: use ceil_char_boundary
            // https://github.com/rust-lang/rust/issues/93743
            // We checked that we're not at the end of a string, and we know the
            // cursor must be on a char boundary, so jump by the length of the
            // next char
            let next_char = self.text[self.cursor..]
                .chars()
                .next()
                .expect("Another char (not at end of string yet)");
            self.cursor += next_char.len_utf8();
        }
    }

    /// Delete character immediately left of the cursor
    fn delete_left(&mut self) {
        if self.is_at_home() {
        } else {
            self.left();
            self.text.remove(self.cursor);
        }
    }

    /// Delete character immediately rightof the cursor
    fn delete_right(&mut self) {
        if self.is_at_end() {
        } else {
            self.text.remove(self.cursor);
        }
    }

    /// Update x scroll to ensure the cursor is visible. This is called on each
    /// render, because that's when we have the width available. Return the new
    /// value
    fn update_scroll(&self, text_stats: TextStats, width: u16) -> u16 {
        // All this math is performed in terms of chars, not bytes. Calculating
        // both cursor offset and text with in chars is O(n) because we have
        // to count the width of each char. This component is designed for
        // relatively short text though, so this shouldn't be an issue
        let cursor_offset = text_stats.cursor_offset as u16;
        let max_scroll =
            (text_stats.text_width as u16 + 1).saturating_sub(width);
        let scroll_x = self.scroll_x.get();
        let new_scroll_x = if cursor_offset < scroll_x {
            // Scroll left so the cursor is at the left edge
            cursor_offset
        } else if cursor_offset >= scroll_x + width {
            // Scroll right so the cursor is at right edge
            cursor_offset - width + 1
        } else if scroll_x > max_scroll {
            // Scroll extends beyond the end of the text, probably because we
            // deleted text from the end. Clamp to the end
            max_scroll
        } else {
            // Cursor is in view already, no change
            scroll_x
        };
        self.scroll_x.set(new_scroll_x);
        new_scroll_x
    }

    /// Get the **character** cursor offset and text width
    fn text_stats(&self) -> TextStats {
        let cursor_offset = self.text[..self.cursor].chars().count();
        let text_width = self.text.chars().count();
        TextStats {
            text_width,
            cursor_offset,
        }
    }
}

/// Cached **character**-based stats for the text. We can pass this around to
/// prevent having to calculate character-based stuff multiple times in one
/// render.
#[derive(Copy, Clone)]
struct TextStats {
    text_width: usize,
    cursor_offset: usize,
}

/*
TODO uncomment
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{layout::Margin, text::Span};
    use rstest::rstest;

    /// Create a span styled as text in the box
    fn text(text: &str) -> Span<'_> {
        Span::styled(text, ViewContext::styles().text_box.text)
    }

    /// Assert that text state matches text/cursor location. Cursor location is
    /// a *character* offset, not byte offset
    #[track_caller]
    fn assert_state(state: &TextState, text: &str, cursor: usize) {
        assert_eq!(state.text, text, "Text does not match");
        assert_eq!(
            state.text_stats().cursor_offset,
            cursor,
            "Cursor character offset does not match"
        );
    }

    /// Test the basic interaction loop on the text box
    #[rstest]
    fn test_interaction(#[with(10, 1)] mut harness: TestHarness) {
        let mut component = TestComponent::new(
            &mut harness,
            TextBox::default().subscribe([
                TextBoxEvent::Cancel,
                TextBoxEvent::Change,
                TextBoxEvent::Submit,
            ]),
        );

        // Assert initial state/view
        assert_state(&component.state, "", 0);
        harness.assert_buffer_lines([vec![text("          ")]]);
        harness.assert_cursor_position((0, 0));

        // Type some text
        component
            .int(&mut harness)
            .send_text("hi!")
            .assert()
            .emitted([
                TextBoxEvent::Change,
                TextBoxEvent::Change,
                TextBoxEvent::Change,
            ]);
        assert_state(&component.state, "hi!", 3);
        harness.assert_buffer_lines([vec![text("hi!"), text("       ")]]);
        harness.assert_cursor_position((3, 0));

        // Sending with a modifier applied should do nothing, unless it's shift
        component
            .int(&mut harness)
            .send_key_modifiers(KeyModifiers::SHIFT, KeyCode::Char('W'))
            .assert()
            .emitted([TextBoxEvent::Change]);
        assert_state(&component.state, "hi!W", 4);
        assert_matches!(
            component
                .int(&mut harness)
                .send_key_modifiers(
                    // This is what crossterm actually sends
                    KeyModifiers::CTRL | KeyModifiers::SHIFT,
                    KeyCode::Char('W'),
                )
                .into_propagated(),
            [Message::Event(Event::Input { .. })]
        );
        assert_state(&component.state, "hi!W", 4);

        // Test emitted events
        component
            .int(&mut harness)
            .send_key(KeyCode::Enter)
            .assert()
            .emitted([TextBoxEvent::Submit]);

        component
            .int(&mut harness)
            .send_key(KeyCode::Esc)
            .assert()
            .emitted([TextBoxEvent::Cancel]);
    }

    /// Test text navigation and deleting. [TextState] has its own tests so
    /// we're mostly just testing that keys are mapped correctly
    #[rstest]
    fn test_navigation(#[with(10, 1)] mut harness: TestHarness) {
        let mut component = TestComponent::new(
            &mut harness,
            TextBox::default().subscribe([TextBoxEvent::Change]),
        );

        // Type some text
        component
            .int(&mut harness)
            .send_text("hello!")
            .assert()
            .emitted([
                // One change event per letter
                TextBoxEvent::Change,
                TextBoxEvent::Change,
                TextBoxEvent::Change,
                TextBoxEvent::Change,
                TextBoxEvent::Change,
                TextBoxEvent::Change,
            ]);
        assert_state(&component.state, "hello!", 6);

        // Move around, delete some text.
        component
            .int(&mut harness)
            .send_key(KeyCode::Left)
            .assert()
            .empty();
        assert_state(&component.state, "hello!", 5);

        component
            .int(&mut harness)
            .send_key(KeyCode::Backspace)
            .assert()
            .emitted([TextBoxEvent::Change]);
        assert_state(&component.state, "hell!", 4);

        component
            .int(&mut harness)
            .send_key(KeyCode::Delete)
            .assert()
            .emitted([TextBoxEvent::Change]);
        assert_state(&component.state, "hell", 4);

        component
            .int(&mut harness)
            .send_key(KeyCode::Home)
            .assert()
            .empty();
        assert_state(&component.state, "hell", 0);

        component
            .int(&mut harness)
            .send_key(KeyCode::Right)
            .assert()
            .empty();
        assert_state(&component.state, "hell", 1);

        component
            .int(&mut harness)
            .send_key(KeyCode::End)
            .assert()
            .empty();
        assert_state(&component.state, "hell", 4);
    }

    /// Test text navigation and deleting. [TextState] has its own tests so
    /// we're mostly just testing that keys are mapped correctly
    #[rstest]
    fn test_scroll(#[with(3, 3)] mut harness: TestHarness) {
        // Leave vertical margin for the scroll bar
        let area = harness.terminal_area().inner(Margin {
            horizontal: 0,
            vertical: 1,
        });
        let mut component = TestComponent::builder(
            &mut harness,
            TextBox::default().subscribe([TextBoxEvent::Change]),
        )
        .with_default_props()
        .with_area(area)
        .build();

        // Type some text
        component
            .int(&mut harness)
            .send_text("012345")
            .assert()
            .emitted([
                // One change event per letter
                TextBoxEvent::Change,
                TextBoxEvent::Change,
                TextBoxEvent::Change,
                TextBoxEvent::Change,
                TextBoxEvent::Change,
                TextBoxEvent::Change,
            ]);
        // End of the string is visible
        harness.assert_buffer_lines(["   ".into(), text("45 "), "◀■▶".into()]);
        harness.assert_cursor_position((2, 1));

        // Deleting from the end should scroll left
        component
            .int(&mut harness)
            .send_key(KeyCode::Backspace)
            .assert()
            .emitted([TextBoxEvent::Change]);
        harness.assert_buffer_lines(["   ".into(), text("34 "), "◀■▶".into()]);
        harness.assert_cursor_position((2, 1));

        // Back to the beginning
        component
            .int(&mut harness)
            .send_key(KeyCode::Home)
            .assert()
            .empty();
        harness.assert_buffer_lines(["   ".into(), text("012"), "◀■▶".into()]);
        harness.assert_cursor_position((0, 1));

        // Scroll shouldn't move until the cursor gets off screen
        component
            .int(&mut harness)
            .send_keys([KeyCode::Right, KeyCode::Right])
            .assert()
            .empty();
        harness.assert_buffer_lines(["   ".into(), text("012"), "◀■▶".into()]);
        harness.assert_cursor_position((2, 1));

        // Push the scroll over
        component
            .int(&mut harness)
            .send_key(KeyCode::Right)
            .assert()
            .empty();
        harness.assert_buffer_lines(["   ".into(), text("123"), "◀■▶".into()]);
        harness.assert_cursor_position((2, 1));

        // Move back doesn't scroll left yet
        component
            .int(&mut harness)
            .send_key(KeyCode::Left)
            .assert()
            .empty();
        harness.assert_buffer_lines(["   ".into(), text("123"), "◀■▶".into()]);
        harness.assert_cursor_position((1, 1));
    }

    #[rstest]
    fn test_sensitive(#[with(3, 1)] mut harness: TestHarness) {
        let mut component = TestComponent::new(
            &mut harness,
            TextBox::default()
                .sensitive(true)
                .subscribe([TextBoxEvent::Change]),
        );

        component
            .int(&mut harness)
            .send_text("hi")
            .assert()
            .emitted([TextBoxEvent::Change, TextBoxEvent::Change]);

        assert_state(&component.state, "hi", 2);
        harness.assert_buffer_lines([text("•• ")]);
        harness.assert_cursor_position((2, 0));
    }

    #[rstest]
    fn test_placeholder(#[with(6, 1)] mut harness: TestHarness) {
        let component = TestComponent::new(
            &mut harness,
            TextBox::default().placeholder("hello"),
        );

        assert_state(&component.state, "", 0);
        let styles = ViewContext::styles().text_box;
        harness.assert_buffer_lines([vec![
            Span::styled("hello", styles.text.patch(styles.placeholder)),
            text(" "),
        ]]);
        harness.assert_cursor_position((0, 0));
    }

    #[rstest]
    fn test_placeholder_focused(#[with(9, 1)] mut harness: TestHarness) {
        let mut component = TestComponent::new(
            &mut harness,
            TextBox::default()
                .placeholder("unfocused")
                .placeholder_focused("focused"),
        );
        let styles = ViewContext::styles().text_box;

        // Focused
        assert_state(&component.state, "", 0);
        harness.assert_buffer_lines([vec![
            Span::styled("focused", styles.text.patch(styles.placeholder)),
            text("  "),
        ]]);
        harness.assert_cursor_position((0, 0));

        // Unfocused
        component.unfocus();
        component.int(&mut harness).drain_draw().assert().empty();
        harness.assert_buffer_lines([vec![Span::styled(
            "unfocused",
            styles.text.patch(styles.placeholder),
        )]]);
    }

    #[rstest]
    fn test_validator(#[with(6, 1)] mut harness: TestHarness) {
        let mut component = TestComponent::new(
            &mut harness,
            TextBox::default()
                .validator(|text| text.len() <= 2)
                .subscribe([TextBoxEvent::Change, TextBoxEvent::Submit]),
        );

        // Valid text, everything is normal
        component
            .int(&mut harness)
            .send_text("he")
            .assert()
            .emitted([TextBoxEvent::Change, TextBoxEvent::Change]);
        harness.assert_buffer_lines([text("he    ")]);
        harness.assert_cursor_position((2, 0));

        component
            .int(&mut harness)
            .send_key(KeyCode::Enter)
            .assert()
            .emitted([TextBoxEvent::Submit]);

        // Invalid text, styling changes and no events are emitted
        component
            .int(&mut harness)
            .send_text("llo")
            .assert()
            .emitted([]);
        harness.assert_buffer_lines([Span::styled(
            "hello ",
            ViewContext::styles().text_box.invalid,
        )]);
        harness.assert_cursor_position((5, 0));
        component
            .int(&mut harness)
            .send_key(KeyCode::Enter)
            .assert()
            .emitted([]);
    }

    #[test]
    fn test_state_insert() {
        let mut state = TextState::default();
        state.insert('a');
        state.insert('b');
        state.left();
        state.insert('c');
        assert_state(&state, "acb", 2);

        state.home();
        state.insert('h');
        state.end();
        state.insert('e');
        assert_state(&state, "hacbe", 5);
    }

    #[test]
    fn test_state_delete() {
        let mut state = TextState {
            text: "abcde".into(),
            ..TextState::default()
        };

        // does nothing
        state.delete_left();
        assert_state(&state, "abcde", 0);

        state.delete_right();
        assert_state(&state, "bcde", 0);

        state.right();
        state.delete_left();
        assert_state(&state, "cde", 0);

        // does nothing
        state.end();
        state.delete_right();
        assert_state(&state, "cde", 3);

        state.delete_left();
        assert_state(&state, "cd", 2);
    }

    /// Test characters that contain multiple bytes
    #[test]
    fn test_state_multibyte_char() {
        let mut state = TextState {
            text: "äëõß".into(),
            ..TextState::default()
        };
        state.delete_right();
        state.end();
        state.delete_left();
        assert_state(&state, "ëõ", 2);

        state.left();
        state.insert('ü');
        assert_state(&state, "ëüõ", 2);
    }
}
*/
