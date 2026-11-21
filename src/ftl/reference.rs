use std::io;
use std::ops::Range;

use super::FlashImage;
use crate::Trace;

impl FlashImage {
    pub(super) fn publish_mapping(&mut self, lba: usize, page: usize) -> Option<usize> {
        self.map.replace(lba, page)
    }

    pub(super) fn relocate_live_pages(
        &mut self,
        victim_pages: Range<usize>,
        trace: &mut Trace,
    ) -> io::Result<()> {
        for lba in 0..self.map.len() {
            if let Some(source) = self.map[lba].filter(|page| victim_pages.contains(page)) {
                let destination = self
                    .nand
                    .erased_pages()
                    .find(|page| !victim_pages.contains(page))
                    .expect("space reserved outside victim");
                self.nand.copy_page(source, destination)?;
                self.map.replace(lba, destination);
                trace.push(format!(
                    "ftl    gc copy LBA {lba} from page {source} to page {destination}"
                ));
            }
        }
        Ok(())
    }
}
