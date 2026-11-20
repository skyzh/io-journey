use std::fmt;
use std::io;

use crate::nand::{
    erase_block_pages, Block, Nand, BLOCK_BYTES, ERASED_BYTE, ERASE_BLOCKS, PAGES_PER_ERASE_BLOCK,
};
use crate::Trace;

pub(crate) const LOGICAL_BLOCKS: usize = 8;

/// The physical pages and mapping that survive a simulated cut.
#[derive(Clone)]
pub(crate) struct FlashImage {
    nand: Nand,
    map: [Option<usize>; LOGICAL_BLOCKS],
}

impl FlashImage {
    pub(crate) fn physical_page(&self, page: usize) -> Option<&Block> {
        self.nand.read(page)
    }

    pub(crate) fn mapped_page(&self, lba: usize) -> Option<usize> {
        self.map.get(lba).copied().flatten()
    }
}

#[derive(Debug)]
pub(crate) struct TornProgram {
    pub(crate) lba: usize,
    pub(crate) page: usize,
    pub(crate) bytes: usize,
}

impl fmt::Display for TornProgram {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "torn NAND program at LBA {}, page {}, after {} of {} bytes",
            self.lba, self.page, self.bytes, BLOCK_BYTES
        )
    }
}

impl std::error::Error for TornProgram {}

pub(crate) struct Ftl {
    live: FlashImage,
    durable: FlashImage,
    tear_after: Option<usize>,
}

impl Ftl {
    pub(crate) fn new() -> Self {
        Self::open(FlashImage {
            nand: Nand::new(),
            map: [None; LOGICAL_BLOCKS],
        })
    }

    pub(crate) fn open(image: FlashImage) -> Self {
        Self {
            live: image.clone(),
            durable: image,
            tear_after: None,
        }
    }

    pub(crate) fn read(&self, lba: usize, trace: &mut Trace) -> io::Result<Block> {
        check_lba(lba)?;
        match self.live.map[lba] {
            Some(page) => {
                trace.push(format!("ftl    read LBA {lba} from page {page}"));
                Ok(self.live.nand.read(page).expect("mapped page").clone())
            }
            None => {
                trace.push(format!("ftl    read LBA {lba} as zero (unmapped)"));
                Ok(Box::new([0; BLOCK_BYTES]))
            }
        }
    }

    pub(crate) fn write(&mut self, lba: usize, data: Block, trace: &mut Trace) -> io::Result<()> {
        check_lba(lba)?;
        if let Some(bytes) = self.tear_after.take() {
            return self.tear_program(lba, &data, bytes, trace);
        }

        // An erase block's worth of free pages leaves room to move live data
        // out of a victim before erasing it.
        if self.live.nand.erased_pages().count() <= PAGES_PER_ERASE_BLOCK {
            self.collect(trace)?;
        }
        let page = self.live.nand.erased_pages().next().expect("free page");
        self.live.nand.program(page, data)?;
        trace.push(format!("nand   program page {page}"));
        let previous = self.live.map[lba].replace(page);
        if let Some(previous) = previous {
            trace.push(format!(
                "ftl    map LBA {lba} -> page {page}, stale page {previous}"
            ));
        } else {
            trace.push(format!("ftl    map LBA {lba} -> page {page}"));
        }
        Ok(())
    }

    pub(crate) fn flush(&mut self, trace: &mut Trace) {
        self.durable = self.live.clone();
        trace.push("ftl    flush published NAND pages and mapping in RAM".into());
    }

    pub(crate) fn power_cut(self) -> FlashImage {
        self.durable
    }

    pub(crate) fn tear_next_program(&mut self, bytes: usize) -> io::Result<()> {
        if !(1..BLOCK_BYTES).contains(&bytes) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "torn program must write a proper, nonempty prefix",
            ));
        }
        self.tear_after = Some(bytes);
        Ok(())
    }

    fn tear_program(
        &mut self,
        lba: usize,
        data: &[u8; BLOCK_BYTES],
        bytes: usize,
        trace: &mut Trace,
    ) -> io::Result<()> {
        // Tear a fresh page. A page freed only in the volatile view could
        // still hold live data in the last committed mapping.
        let page = self
            .live
            .nand
            .erased_pages()
            .find(|&page| self.durable.nand.read(page).is_none())
            .ok_or_else(|| io::Error::other("torn program needs a page erased in both images"))?;
        let mut partial = Box::new([ERASED_BYTE; BLOCK_BYTES]);
        partial[..bytes].copy_from_slice(&data[..bytes]);
        self.live.nand.program(page, partial.clone())?;
        self.durable.nand.program(page, partial)?;

        let tear = TornProgram { lba, page, bytes };
        trace.push(format!("nand   {tear}"));
        Err(io::Error::other(tear))
    }

    fn collect(&mut self, trace: &mut Trace) -> io::Result<()> {
        let free = self.live.nand.erased_pages().count();
        let mut victim = None;
        let mut most_stale = 0;
        for erase_block in 0..ERASE_BLOCKS {
            let pages = erase_block_pages(erase_block);
            let used = pages
                .clone()
                .filter(|&page| self.live.nand.read(page).is_some())
                .count();
            let mapped = self
                .live
                .map
                .iter()
                .flatten()
                .filter(|page| pages.contains(page))
                .count();
            let stale = used - mapped;
            let free_outside = free - (PAGES_PER_ERASE_BLOCK - used);
            if stale > most_stale && mapped <= free_outside {
                victim = Some(erase_block);
                most_stale = stale;
            }
        }
        let victim = victim.ok_or_else(|| io::Error::other("no reclaimable NAND erase block"))?;
        let victim_pages = erase_block_pages(victim);

        for (lba, mapping) in self.live.map.iter_mut().enumerate() {
            if let Some(source) = mapping.filter(|page| victim_pages.contains(page)) {
                let destination = self
                    .live
                    .nand
                    .erased_pages()
                    .find(|page| !victim_pages.contains(page))
                    .expect("space reserved outside victim");
                let data = self.live.nand.read(source).expect("mapped page").clone();
                self.live.nand.program(destination, data)?;
                *mapping = Some(destination);
                trace.push(format!(
                    "ftl    gc copy LBA {lba} from page {source} to page {destination}"
                ));
            }
        }
        self.live.nand.erase(victim);
        trace.push(format!(
            "nand   erase block {victim}, reclaim {most_stale} stale pages"
        ));
        Ok(())
    }
}

pub(crate) fn check_lba(lba: usize) -> io::Result<()> {
    if lba >= LOGICAL_BLOCKS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("LBA {lba} out of range"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_copies_live_pages_before_erasing_and_recovery_keeps_them() {
        let mut trace = Vec::new();
        let mut ftl = Ftl::new();
        for lba in 0..LOGICAL_BLOCKS {
            ftl.write(lba, Box::new([lba as u8; BLOCK_BYTES]), &mut trace)
                .unwrap();
        }
        for value in 10..=13 {
            ftl.write(0, Box::new([value; BLOCK_BYTES]), &mut trace)
                .unwrap();
        }
        ftl.write(1, Box::new([42; BLOCK_BYTES]), &mut trace)
            .unwrap();
        assert!(trace.iter().any(|event| event.contains("gc copy LBA 0")));
        ftl.flush(&mut trace);

        let ftl = Ftl::open(ftl.power_cut());
        for (lba, value) in [13, 42, 2, 3, 4, 5, 6, 7].into_iter().enumerate() {
            assert_eq!(*ftl.read(lba, &mut trace).unwrap(), [value; BLOCK_BYTES]);
        }
    }

    #[test]
    fn a_torn_program_persists_the_exact_prefix_without_publishing_a_mapping() {
        let mut trace = Vec::new();
        let mut ftl = Ftl::new();
        ftl.write(1, Box::new([0x12; BLOCK_BYTES]), &mut trace)
            .unwrap();
        ftl.flush(&mut trace);
        ftl.tear_next_program(19).unwrap();
        let error = ftl
            .write(1, Box::new([0x34; BLOCK_BYTES]), &mut trace)
            .unwrap_err();
        let tear = error
            .get_ref()
            .unwrap()
            .downcast_ref::<TornProgram>()
            .unwrap();
        let image = ftl.power_cut();
        assert_eq!(image.mapped_page(1), Some(0));
        let page = image.physical_page(tear.page).unwrap();
        assert_eq!(page[..19], [0x34; 19]);
        assert!(page[19..].iter().all(|&byte| byte == ERASED_BYTE));
        assert_eq!(
            *Ftl::open(image).read(1, &mut trace).unwrap(),
            [0x12; BLOCK_BYTES]
        );
    }
}
