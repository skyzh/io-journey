use std::fmt;
use std::io;

use crate::nand::{
    Block, Nand, NandGeometry, BLOCK_BYTES, ERASED_BYTE, ERASE_BLOCKS, PAGES_PER_ERASE_BLOCK,
};
use crate::Trace;

#[path = "ftl/mapping.rs"]
mod mapping;
use mapping::Mapping;

#[cfg(not(feature = "learner"))]
#[path = "ftl/latency.rs"]
pub mod latency;

#[cfg_attr(feature = "learner", path = "../course/learner/ftl.rs")]
#[cfg_attr(not(feature = "learner"), path = "ftl/reference.rs")]
mod implementation;

pub(crate) const LOGICAL_BLOCKS: usize = 8;

/// The physical pages and mapping that survive a simulated cut.
#[derive(Clone)]
pub struct FlashImage {
    nand: Nand,
    map: Mapping,
}

impl FlashImage {
    pub fn physical_page(&self, page: usize) -> Option<&Block> {
        self.nand.read(page)
    }

    pub fn mapped_page(&self, lba: usize) -> Option<usize> {
        self.map.location(lba)
    }
}

#[derive(Debug)]
pub struct TornProgram {
    pub lba: usize,
    pub page: usize,
    pub bytes: usize,
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

pub struct Ftl {
    live: FlashImage,
    durable: FlashImage,
    tear: Option<ProgramFault>,
}

struct ProgramFault {
    lba: usize,
    prefix_bytes: usize,
}

impl Ftl {
    pub fn new() -> Self {
        let geometry =
            NandGeometry::new(ERASE_BLOCKS, PAGES_PER_ERASE_BLOCK).expect("demo geometry");
        Self::with_geometry(LOGICAL_BLOCKS, geometry).expect("demo device")
    }

    pub fn with_geometry(logical_blocks: usize, geometry: NandGeometry) -> io::Result<Self> {
        // At the reserve, at least one occupied page must be stale.
        if !(1..geometry.physical_pages() - geometry.pages_per_erase_block())
            .contains(&logical_blocks)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "logical capacity must leave an erase block and one extra physical page",
            ));
        }
        let map = Mapping::new(logical_blocks)?;
        Ok(Self::from_image(FlashImage {
            nand: Nand::new(geometry)?,
            map,
        }))
    }

    pub fn open(mut image: FlashImage, trace: &mut Trace) -> io::Result<Self> {
        // An interrupted program can spend one of the reserved pages.
        // Reclaim before cloning so both copies have room for the next cut.
        if image.nand.free_count() < image.nand.geometry().pages_per_erase_block() {
            image.collect(trace)?;
            trace.push("ftl    restored the erase-block reserve on reopen".into());
        }
        Ok(Self::from_image(image))
    }

    fn from_image(image: FlashImage) -> Self {
        Self {
            live: image.clone(),
            durable: image,
            tear: None,
        }
    }

    pub(crate) fn logical_blocks(&self) -> usize {
        self.live.map.len()
    }

    fn check_lba(&self, lba: usize) -> io::Result<()> {
        if lba >= self.logical_blocks() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("LBA {lba} out of range"),
            ));
        }
        Ok(())
    }

    pub(crate) fn read(&self, lba: usize, trace: &mut Trace) -> io::Result<Block> {
        self.check_lba(lba)?;
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
        self.check_lba(lba)?;
        if self.tear.as_ref().is_some_and(|fault| fault.lba == lba) {
            let fault = self.tear.take().expect("armed program");
            return self.tear_program(lba, &data, fault.prefix_bytes, trace);
        }

        // An erase block's worth of free pages leaves room to move live data
        // out of a victim before erasing it.
        if self.live.nand.free_count() <= self.live.nand.geometry().pages_per_erase_block() {
            self.live.collect(trace)?;
        }
        let page = self.live.nand.erased_pages().next().expect("free page");
        self.live.nand.program(page, data)?;
        trace.push(format!("nand   program page {page}"));
        let previous = self.live.publish_mapping(lba, page);
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

    pub(crate) fn checkpoint(&self) -> FlashImage {
        self.durable.clone()
    }

    pub(crate) fn arm_torn_write(&mut self, lba: usize, prefix_bytes: usize) -> io::Result<()> {
        self.check_lba(lba)?;
        if !(1..BLOCK_BYTES).contains(&prefix_bytes) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "torn program must write a proper, nonempty prefix",
            ));
        }
        self.tear = Some(ProgramFault { lba, prefix_bytes });
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
}

impl FlashImage {
    fn collect(&mut self, trace: &mut Trace) -> io::Result<()> {
        let geometry = self.nand.geometry();
        let free = self.nand.free_count();
        let mut victim = None;
        let mut most_stale = 0;
        for erase_block in 0..geometry.erase_blocks() {
            let pages = geometry.erase_block_pages(erase_block);
            let used = self.nand.programmed_in(erase_block);
            let mapped = self.map.live_in(&pages);
            let stale = used - mapped;
            let free_outside = free - (geometry.pages_per_erase_block() - used);
            if stale > most_stale && mapped <= free_outside {
                victim = Some(erase_block);
                most_stale = stale;
            }
        }
        let victim = victim.ok_or_else(|| io::Error::other("no reclaimable NAND erase block"))?;
        let victim_pages = geometry.erase_block_pages(victim);

        self.relocate_live_pages(victim_pages, trace)?;
        self.nand.erase(victim);
        trace.push(format!(
            "nand   erase block {victim}, reclaim {most_stale} stale pages"
        ));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_rejects_zero_overflow_and_a_missing_spare_page() {
        for (blocks, pages) in [(0, 4), (4, 0), (usize::MAX, 2), (usize::MAX, 1)] {
            let error = NandGeometry::new(blocks, pages).err().unwrap();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        }
        let geometry = NandGeometry::new(2, 4).unwrap();
        for logical_blocks in [0, 4, 8] {
            let error = Ftl::with_geometry(logical_blocks, geometry).err().unwrap();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        }
    }

    #[test]
    fn repeated_cuts_restore_the_reserve_at_the_geometry_limit() {
        let mut trace = Vec::new();
        for (erase_blocks, reserve, logical_blocks) in [(3, 1, 1), (2, 4, 3), (5, 16, 63)] {
            let geometry = NandGeometry::new(erase_blocks, reserve).unwrap();
            let mut ftl = Ftl::with_geometry(logical_blocks, geometry).unwrap();
            for lba in 0..logical_blocks {
                ftl.write(lba, Box::new([lba as u8; BLOCK_BYTES]), &mut trace)
                    .unwrap();
            }
            while ftl.live.nand.erased_pages().count() > reserve {
                ftl.write(0, Box::new([0; BLOCK_BYTES]), &mut trace)
                    .unwrap();
            }
            ftl.flush(&mut trace);

            for _ in 0..reserve + 2 {
                ftl.arm_torn_write(0, 37).unwrap();
                let error = ftl
                    .write(0, Box::new([0x80; BLOCK_BYTES]), &mut trace)
                    .unwrap_err();
                assert!(error.get_ref().unwrap().is::<TornProgram>());
                ftl = Ftl::open(ftl.power_cut(), &mut trace).unwrap();
                assert!(ftl.durable.nand.erased_pages().count() >= reserve);
                for lba in 0..logical_blocks {
                    assert_eq!(
                        *ftl.read(lba, &mut trace).unwrap(),
                        [lba as u8; BLOCK_BYTES]
                    );
                }
            }

            ftl.write(0, Box::new([0x91; BLOCK_BYTES]), &mut trace)
                .unwrap();
            ftl.flush(&mut trace);
            let reopened = Ftl::open(ftl.power_cut(), &mut trace).unwrap();
            assert_eq!(*reopened.read(0, &mut trace).unwrap(), [0x91; BLOCK_BYTES]);
        }
    }

    #[test]
    fn a_torn_write_requires_an_erased_page_in_both_images() {
        let mut trace = Vec::new();
        let geometry = NandGeometry::new(4, 1).unwrap();
        let mut ftl = Ftl::with_geometry(2, geometry).unwrap();
        for (lba, byte) in [(0, 1), (1, 2), (0, 3)] {
            ftl.write(lba, Box::new([byte; BLOCK_BYTES]), &mut trace)
                .unwrap();
        }
        ftl.flush(&mut trace);
        ftl.arm_torn_write(0, 19).unwrap();
        let error = ftl
            .write(0, Box::new([4; BLOCK_BYTES]), &mut trace)
            .unwrap_err();
        assert!(error.get_ref().unwrap().is::<TornProgram>());

        let mut ftl = Ftl::open(ftl.power_cut(), &mut trace).unwrap();
        ftl.arm_torn_write(1, 19).unwrap();
        ftl.write(0, Box::new([5; BLOCK_BYTES]), &mut trace)
            .unwrap();
        let error = ftl
            .write(1, Box::new([6; BLOCK_BYTES]), &mut trace)
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "torn program needs a page erased in both images"
        );

        let reopened = Ftl::open(ftl.power_cut(), &mut trace).unwrap();
        assert_eq!(*reopened.read(0, &mut trace).unwrap(), [3; BLOCK_BYTES]);
        assert_eq!(*reopened.read(1, &mut trace).unwrap(), [2; BLOCK_BYTES]);
    }

    #[test]
    fn successful_programs_publish_the_new_mapping() {
        let mut trace = Vec::new();
        let mut ftl = Ftl::new();
        ftl.write(0, Box::new([0x12; BLOCK_BYTES]), &mut trace)
            .unwrap();
        let first = ftl
            .live
            .mapped_page(0)
            .expect("map the first programmed page");
        assert_eq!(ftl.read(0, &mut trace).unwrap()[0], 0x12);

        ftl.write(0, Box::new([0x34; BLOCK_BYTES]), &mut trace)
            .unwrap();
        let second = ftl.live.mapped_page(0).expect("map the replacement page");
        assert_ne!(first, second);
        assert_eq!(ftl.live.nand.read(first).unwrap()[0], 0x12);
        assert_eq!(ftl.read(0, &mut trace).unwrap()[0], 0x34);
        assert!(trace.contains(&format!(
            "ftl    map LBA 0 -> page {second}, stale page {first}"
        )));
    }

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
        let old_page = ftl.live.mapped_page(0).expect("map the latest page");
        ftl.write(1, Box::new([42; BLOCK_BYTES]), &mut trace)
            .unwrap();
        assert_ne!(ftl.live.mapped_page(0), Some(old_page));
        assert!(ftl.live.nand.read(old_page).is_none());
        ftl.flush(&mut trace);

        let ftl = Ftl::open(ftl.power_cut(), &mut trace).unwrap();
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
        ftl.arm_torn_write(1, 19).unwrap();
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
            *Ftl::open(image, &mut trace)
                .unwrap()
                .read(1, &mut trace)
                .unwrap(),
            [0x12; BLOCK_BYTES]
        );
    }

    #[test]
    fn exported_images_preserve_bytes_through_collection_and_tear() {
        let mut trace = Vec::new();
        let mut ftl = Ftl::new();
        for lba in 0..8 {
            ftl.write(lba, Box::new([lba as u8; BLOCK_BYTES]), &mut trace)
                .unwrap();
        }
        for value in 10..=13 {
            ftl.write(0, Box::new([value; BLOCK_BYTES]), &mut trace)
                .unwrap();
        }
        ftl.flush(&mut trace);
        let first = ftl.checkpoint();
        let old = first.mapped_page(0).unwrap();
        ftl.write(1, Box::new([42; BLOCK_BYTES]), &mut trace)
            .unwrap();
        let relocated = ftl.live.mapped_page(0).unwrap();
        assert!(ftl.live.nand.read(old).is_none());
        assert!(first.physical_page(relocated).is_none());
        assert_eq!(**first.physical_page(old).unwrap(), [13; BLOCK_BYTES]);

        ftl.flush(&mut trace);
        let last = ftl.checkpoint();
        ftl.arm_torn_write(1, 19).unwrap();
        let error = ftl
            .write(1, Box::new([0x73; BLOCK_BYTES]), &mut trace)
            .unwrap_err();
        let tear = error
            .get_ref()
            .unwrap()
            .downcast_ref::<TornProgram>()
            .unwrap();
        let crash = ftl.power_cut();
        assert!(last.physical_page(tear.page).is_none());
        assert_eq!(crash.mapped_page(1), last.mapped_page(1));
        let partial = crash.physical_page(tear.page).unwrap();
        assert_eq!(partial[..19], [0x73; 19]);
        assert!(partial[19..].iter().all(|&byte| byte == ERASED_BYTE));
        let saved = Ftl::open(first, &mut trace).unwrap();
        assert_eq!(*saved.read(1, &mut trace).unwrap(), [1; BLOCK_BYTES]);
        let recovered = Ftl::open(crash, &mut trace).unwrap();
        for (lba, byte) in [13, 42, 2, 3, 4, 5, 6, 7].into_iter().enumerate() {
            assert_eq!(
                *recovered.read(lba, &mut trace).unwrap(),
                [byte; BLOCK_BYTES]
            );
        }
    }

    #[test]
    fn repeated_torn_programs_leave_room_for_a_new_durable_write() {
        let mut trace = Vec::new();
        let mut ftl = Ftl::new();
        // Leave four erased pages and at least one live page in each erase
        // block. With no free pages, no whole block could be erased safely.
        for (value, lba) in [0, 1, 2, 3, 4, 5, 6, 7, 0, 0, 0, 0, 1, 1, 1]
            .into_iter()
            .enumerate()
        {
            ftl.write(lba, Box::new([value as u8; BLOCK_BYTES]), &mut trace)
                .unwrap();
        }
        ftl.flush(&mut trace);
        let mut committed: [Block; LOGICAL_BLOCKS] =
            std::array::from_fn(|lba| ftl.read(lba, &mut trace).unwrap());

        for _ in 0..4 {
            ftl.arm_torn_write(1, 37).unwrap();
            let error = ftl
                .write(1, Box::new([0x80; BLOCK_BYTES]), &mut trace)
                .unwrap_err();
            assert!(error.get_ref().unwrap().is::<TornProgram>());
            ftl = Ftl::open(ftl.power_cut(), &mut trace).unwrap();
            for (lba, block) in committed.iter().enumerate() {
                assert_eq!(&ftl.read(lba, &mut trace).unwrap(), block);
            }
        }

        committed[1] = Box::new([0x91; BLOCK_BYTES]);
        ftl.write(1, committed[1].clone(), &mut trace)
            .expect("the reopened FTL should accept another write");
        ftl.flush(&mut trace);
        let ftl = Ftl::open(ftl.power_cut(), &mut trace).unwrap();
        for (lba, block) in committed.iter().enumerate() {
            assert_eq!(&ftl.read(lba, &mut trace).unwrap(), block);
        }
    }
}
