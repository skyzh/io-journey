use std::io;
use std::ops::Range;

use super::FlashImage;
use crate::Trace;

impl FlashImage {
    pub(super) fn publish_mapping(&mut self, lba: usize, page: usize) -> Option<usize> {
        self.map[lba].replace(page)
    }

    pub(super) fn relocate_live_pages(
        &mut self,
        victim_pages: Range<usize>,
        trace: &mut Trace,
    ) -> io::Result<()> {
        for (lba, mapping) in self.map.iter_mut().enumerate() {
            if let Some(source) = mapping.filter(|page| victim_pages.contains(page)) {
                let destination = self
                    .nand
                    .erased_pages()
                    .find(|page| !victim_pages.contains(page))
                    .expect("space reserved outside victim");
                let data = self.nand.read(source).expect("mapped page").clone();
                self.nand.program(destination, data)?;
                *mapping = Some(destination);
                trace.push(format!(
                    "ftl    gc copy LBA {lba} from page {source} to page {destination}"
                ));
            }
        }
        Ok(())
    }
}
