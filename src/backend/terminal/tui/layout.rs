//! Cached TUI layout calculation

use ratatui::layout::{Layout, Rect};
use std::{
    cell::RefCell,
    collections::{HashMap, hash_map::Entry},
};

type Cache = HashMap<(Rect, Layout), Box<[Rect]>>;
thread_local! {
    static CACHE: RefCell<Cache> = RefCell::default();
}

/// Extension trait for [Layout] to enable caching
pub trait LayoutCached {
    /// A version of [Layout::areas] that caches the results
    ///
    /// Calculating layouts can be extremely expensive. In benchmarking, I saw
    /// greater than 96% of CPU time during stepping spent on layout
    /// calculation. Our layouts are pretty static, so recalculating on every
    /// CPU cycle is dumb. This uses a thread-local cache with no eviction.
    ///
    /// A clippy rule prevents direct usage of [Layout::areas]. Always prefer
    /// this method instead.
    fn areas_cached<const N: usize>(&self, area: Rect) -> [Rect; N];
}

impl LayoutCached for Layout {
    fn areas_cached<const N: usize>(&self, area: Rect) -> [Rect; N] {
        CACHE.with_borrow_mut(|cache| {
            match cache.entry((area, self.clone())) {
                Entry::Occupied(entry) => {
                    // Copy the slice out and force it to length N
                    *<&[Rect; N]>::try_from(&**entry.get()).unwrap()
                }
                Entry::Vacant(entry) => {
                    let layout = &entry.key().1;
                    #[expect(clippy::disallowed_methods)]
                    let areas = layout.areas::<N>(area);
                    entry.insert(Box::new(areas));
                    areas
                }
            }
        })
    }
}
