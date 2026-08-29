//! Scrollbar widget

use ratatui::{
    buffer::Buffer,
    layout::{Offset, Rect},
    widgets::{
        ScrollbarOrientation, ScrollbarState as RatScrollbarState,
        StatefulWidget,
    },
};

/// A wrapper around Ratatui's scrollbar to make it more ergonomic. This has a
/// few main purposes:
/// - Standardize styling
/// - Handle margin offsets
/// - Handle annoying state calculation
///
/// The default scrollbar state really isn't good. It doesn't have any utilities
/// for tracking a particular line. [ScrollbarState] handles that easily.
#[derive(Clone, Debug)]
pub struct Scrollbar {
    /// How far should the scrollbar be offset from its content? Positive to
    /// offset out, negative to offset in. Defaults to 1, because most content
    /// has a border that can contain the scrollbar.
    pub margin: i32,
    /// Where is the scrollbar placed?
    pub orientation: ScrollbarOrientation,
}

impl Default for Scrollbar {
    fn default() -> Self {
        Self {
            margin: 1,
            orientation: ScrollbarOrientation::VerticalRight,
        }
    }
}

impl StatefulWidget for Scrollbar {
    type State = ScrollbarState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let (begin_symbol, thumb_symbol, end_symbol) = match &self.orientation {
            ScrollbarOrientation::VerticalRight
            | ScrollbarOrientation::VerticalLeft => ("▲", "█", "▼"),
            ScrollbarOrientation::HorizontalBottom
            | ScrollbarOrientation::HorizontalTop => ("◀", "■", "▶"),
        };

        // Apply an offset to put this outside the content
        let offset = match &self.orientation {
            ScrollbarOrientation::VerticalRight => Offset {
                x: self.margin,
                y: 0,
            },
            ScrollbarOrientation::VerticalLeft => Offset {
                x: -self.margin,
                y: 0,
            },
            ScrollbarOrientation::HorizontalBottom => Offset {
                x: 0,
                y: self.margin,
            },
            ScrollbarOrientation::HorizontalTop => Offset {
                x: 0,
                y: -self.margin,
            },
        };

        let scrollbar =
            ratatui::widgets::Scrollbar::new(self.orientation.clone())
                .begin_symbol(Some(begin_symbol))
                .thumb_symbol(thumb_symbol)
                .end_symbol(Some(end_symbol));

        let area = area.offset(offset);
        // Avoid panic if there's nowhere to render the scroll bar. This can
        // occur if the screen gets really small
        state.set_view_length(area, self.orientation);
        if !area.is_empty() {
            StatefulWidget::render(
                scrollbar,
                area,
                buf,
                &mut (state as &Self::State).into(),
            );
        }
    }
}

/// Widget state for [Scrollbar]
///
/// This assumes each content item is a single line. If that assumptions gets
/// broken, I'll reevaluate.
#[derive(Debug)]
pub struct ScrollbarState {
    /// Number of rows in your content, e.g. items in a list or lines in a
    /// text file. For horizontal scrolling, this is the number of columns.
    content_length: usize,
    /// Length/width of the draw area
    ///
    /// Needed to compute scroll position. Updated on every draw.
    view_length: u16,
    /// Visual offset into the content, i.e. the index of the first visible
    /// item
    offset: usize,
}

impl ScrollbarState {
    /// Initialize new scrollbar state with a content length
    ///
    /// The content length is the number of scrollable items in the view.
    pub fn new(content_length: usize) -> Self {
        Self {
            content_length,
            view_length: 0,
            offset: 0,
        }
    }

    /// Get the current offset
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Update the scroll offset to ensure the given item is visible
    pub fn scroll_to(&mut self, item: usize) {
        self.offset = self.offset.clamp(
            item.saturating_sub(usize::from(self.view_length) - 1),
            item,
        );
    }

    /// Set the height/width of the view port
    ///
    /// This is updated on every frame to keep the scrollbar in sync with
    /// resizes.
    fn set_view_length(
        &mut self,
        area: Rect,
        orientation: ScrollbarOrientation,
    ) {
        self.view_length = match orientation {
            ScrollbarOrientation::VerticalRight
            | ScrollbarOrientation::VerticalLeft => area.height,
            ScrollbarOrientation::HorizontalBottom
            | ScrollbarOrientation::HorizontalTop => area.width,
        };
    }
}

impl From<&ScrollbarState> for RatScrollbarState {
    fn from(state: &ScrollbarState) -> RatScrollbarState {
        // To Ratatui, content_length is how many possible scroll positions
        // there are. 1 for the current viewport + the number of items outside
        // the viewport (on either side).
        //
        // If the entire content fits in the viewport, use 0 to hide the scroll
        let content_length = if state.content_length <= state.view_length.into()
        {
            0
        } else {
            state
                .content_length
                .saturating_sub(state.view_length.into())
                + 1
        };
        // position is the index of the first *visible* element
        RatScrollbarState::new(content_length).position(state.offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{
        buffer::Buffer,
        widgets::{List, ListState},
    };
    use rstest::rstest;

    fn state(content_length: usize, position: usize) -> RatScrollbarState {
        RatScrollbarState::new(content_length).position(position)
    }

    #[rstest]
    // If len <= height, we should have *no* scrollbar
    #[case::empty(0, 0, state(0, 0))]
    #[case::extra_space(9, 0, state(0, 0))]
    #[case::perfect_fit_first(10, 0, state(0, 0))]
    #[case::perfect_fit_last(10, 9, state(0, 0))]
    // Overflow without offset
    #[case::overflow(11, 0, state(2, 0))]
    // We scrolled down, but not far enough to move the scrollbar
    #[case::overflow_offset(11, 3, state(2, 0))]
    // Scroll down far enough to move the scrollbar to an intermediate position
    #[case::overflow_scrolled(12, 10, state(3, 1))]
    // Last item is selected, so items 10-19 are visible
    #[case::overflow_scrolled_bottom(20, 19, state(11, 10))]
    fn test_state(
        #[case] content_length: usize,
        #[case] selected: usize,
        #[case] expected: RatScrollbarState,
    ) {
        let area = Rect::new(0, 0, 5, 10);

        // Render a list once to get a realistic offset calculation
        let mut buffer = Buffer::empty(area);
        let list: List = (0..content_length).map(|i| i.to_string()).collect();
        let mut state = ListState::default().with_selected(Some(selected));
        StatefulWidget::render(list, area, &mut buffer, &mut state);

        let state = ScrollbarState {
            content_length,
            view_length: 10,
            offset: state.offset(),
        };
        assert_eq!(RatScrollbarState::from(&state), expected);
    }
}
