use std::ops::Range;

use super::FreePages;

impl FreePages {
    pub(crate) fn record_program(&mut self, page: usize, block: usize) {
        let removed = self.pages.remove(&page);
        debug_assert!(removed);
        self.used_by_block[block] += 1;
    }

    pub(crate) fn record_erase(&mut self, block: usize, pages: Range<usize>) {
        self.pages.extend(pages);
        self.used_by_block[block] = 0;
    }
}

pub(crate) fn record_remap(
    live_by_block: &mut [usize],
    pages_per_block: usize,
    previous: Option<usize>,
    page: usize,
) {
    if let Some(previous) = previous {
        live_by_block[previous / pages_per_block] -= 1;
    }
    live_by_block[page / pages_per_block] += 1;
}
