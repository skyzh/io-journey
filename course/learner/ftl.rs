use std::io;
use std::ops::Range;

use super::FlashImage;
use crate::Trace;

impl FlashImage {
    pub(super) fn publish_mapping(&mut self, lba: usize, _page: usize) -> Option<usize> {
        // TODO: Publish the new page for this LBA and return the old mapping.
        self.map[lba]
    }

    pub(super) fn relocate_live_pages(
        &mut self,
        _victim_pages: Range<usize>,
        _trace: &mut Trace,
    ) -> io::Result<()> {
        // TODO: Copy each live page out of the victim and update its LBA mapping.
        Err(io::Error::other("implement live-page relocation"))
    }
}
