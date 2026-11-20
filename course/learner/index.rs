use std::ops::Range;

use super::FreePages;

impl FreePages {
    pub(crate) fn record_program(&mut self, _page: usize, _block: usize) {
        // TODO: Remove the programmed page from the free set and count it as used.
    }

    pub(crate) fn record_erase(&mut self, _block: usize, _pages: Range<usize>) {
        // TODO: Make every page in the block free and reset its occupied count.
    }
}

pub(crate) fn record_remap(
    _live_by_block: &mut [usize],
    _pages_per_block: usize,
    _previous: Option<usize>,
    _page: usize,
) {
    // TODO: Move one live reference from the old physical block to the new one.
}
