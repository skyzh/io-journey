use std::io;
use std::ops::Range;

#[cfg(feature = "indexed-ftl")]
use crate::free_space::FreePages;

#[cfg_attr(feature = "learner-sharing", path = "../course/learner/pages.rs")]
#[cfg_attr(not(feature = "learner-sharing"), path = "nand/pages.rs")]
mod storage;
use storage::StoredPage;

#[cfg_attr(feature = "learner-nand", path = "../course/learner/nand.rs")]
#[cfg_attr(not(feature = "learner-nand"), path = "nand/reference.rs")]
mod operations;

pub const BLOCK_BYTES: usize = 4096;
pub(crate) const PAGES_PER_ERASE_BLOCK: usize = 4;
pub(crate) const ERASE_BLOCKS: usize = 4;
pub const ERASED_BYTE: u8 = 0xff;

pub type Block = Box<[u8; BLOCK_BYTES]>;

#[cfg(not(feature = "learner"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PhysicalOp {
    Read(usize),
    Program(usize),
    Erase(usize),
}

#[derive(Clone, Copy)]
pub struct NandGeometry {
    erase_blocks: usize,
    pages_per_erase_block: usize,
    physical_pages: usize,
}

impl NandGeometry {
    pub fn new(erase_blocks: usize, pages_per_erase_block: usize) -> io::Result<Self> {
        let physical_pages = erase_blocks
            .checked_mul(pages_per_erase_block)
            .filter(|&pages| pages > 0 && pages.checked_mul(BLOCK_BYTES).is_some())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid NAND geometry"))?;
        Ok(Self {
            erase_blocks,
            pages_per_erase_block,
            physical_pages,
        })
    }

    pub(crate) fn erase_blocks(self) -> usize {
        self.erase_blocks
    }

    pub(crate) fn pages_per_erase_block(self) -> usize {
        self.pages_per_erase_block
    }

    pub(crate) fn physical_pages(self) -> usize {
        self.physical_pages
    }

    pub(crate) fn erase_block_pages(self, erase_block: usize) -> Range<usize> {
        let start = erase_block * self.pages_per_erase_block;
        start..start + self.pages_per_erase_block
    }
}

#[derive(Clone)]
pub(crate) struct Nand {
    geometry: NandGeometry,
    pages: Vec<Option<StoredPage>>,
    #[cfg(not(feature = "learner"))]
    operations: Option<Vec<PhysicalOp>>,
    #[cfg(feature = "indexed-ftl")]
    free: FreePages,
}

impl Nand {
    pub(crate) fn new(geometry: NandGeometry) -> io::Result<Self> {
        let mut pages = Vec::new();
        pages
            .try_reserve_exact(geometry.physical_pages)
            .map_err(io::Error::other)?;
        pages.resize(geometry.physical_pages, None);
        Ok(Self {
            geometry,
            pages,
            #[cfg(not(feature = "learner"))]
            operations: None,
            #[cfg(feature = "indexed-ftl")]
            free: FreePages::new(geometry)?,
        })
    }

    pub(crate) fn geometry(&self) -> NandGeometry {
        self.geometry
    }

    pub(crate) fn read(&self, page: usize) -> Option<&Block> {
        operations::read_page(&self.pages, page)
    }

    #[cfg(not(feature = "learner"))]
    pub(crate) fn start_capture(&mut self) {
        debug_assert!(self.operations.is_none());
        self.operations = Some(Vec::new());
    }

    #[cfg(not(feature = "learner"))]
    pub(crate) fn finish_capture(&mut self) -> Vec<PhysicalOp> {
        self.operations.take().expect("active NAND capture")
    }

    #[cfg(not(feature = "learner"))]
    pub(crate) fn copy_page(&mut self, source: usize, destination: usize) -> io::Result<()> {
        let data = self.read(source).expect("mapped page").clone();
        self.record(PhysicalOp::Read(source));
        self.program(destination, data)
    }

    #[cfg(not(feature = "learner"))]
    fn record(&mut self, operation: PhysicalOp) {
        if let Some(operations) = &mut self.operations {
            operations.push(operation);
        }
    }

    #[cfg(not(feature = "indexed-ftl"))]
    pub(crate) fn erased_pages(&self) -> impl Iterator<Item = usize> + '_ {
        self.pages
            .iter()
            .enumerate()
            .filter_map(|(page, data)| data.is_none().then_some(page))
    }

    #[cfg(feature = "indexed-ftl")]
    pub(crate) fn erased_pages(&self) -> impl Iterator<Item = usize> + '_ {
        self.free.iter()
    }

    #[cfg(not(feature = "indexed-ftl"))]
    pub(crate) fn free_count(&self) -> usize {
        self.erased_pages().count()
    }

    #[cfg(feature = "indexed-ftl")]
    pub(crate) fn free_count(&self) -> usize {
        self.free.count()
    }

    #[cfg(not(feature = "indexed-ftl"))]
    pub(crate) fn programmed_in(&self, block: usize) -> usize {
        self.geometry
            .erase_block_pages(block)
            .filter(|&page| self.read(page).is_some())
            .count()
    }

    #[cfg(feature = "indexed-ftl")]
    pub(crate) fn programmed_in(&self, block: usize) -> usize {
        self.free.used_in(block)
    }

    pub(crate) fn program(&mut self, page: usize, data: Block) -> io::Result<()> {
        operations::program_page(&mut self.pages, page, data)?;
        #[cfg(feature = "indexed-ftl")]
        self.free
            .record_program(page, page / self.geometry.pages_per_erase_block());
        #[cfg(not(feature = "learner"))]
        self.record(PhysicalOp::Program(page));
        Ok(())
    }

    pub(crate) fn erase(&mut self, erase_block: usize) {
        let pages = self.geometry.erase_block_pages(erase_block);
        operations::erase_pages(&mut self.pages[pages.clone()]);
        #[cfg(feature = "indexed-ftl")]
        self.free.record_erase(erase_block, pages);
        #[cfg(not(feature = "learner"))]
        self.record(PhysicalOp::Erase(erase_block));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn programming_requires_an_erased_page_and_erase_clears_the_whole_block() {
        let geometry = NandGeometry::new(ERASE_BLOCKS, PAGES_PER_ERASE_BLOCK).unwrap();
        let mut nand = Nand::new(geometry).unwrap();
        assert!(nand.read(0).is_none());
        nand.program(0, Box::new([0x12; BLOCK_BYTES])).unwrap();
        nand.program(1, Box::new([0x34; BLOCK_BYTES])).unwrap();
        nand.program(4, Box::new([0x56; BLOCK_BYTES])).unwrap();
        assert!(nand.program(0, Box::new([0x78; BLOCK_BYTES])).is_err());
        assert_eq!(nand.read(0).unwrap()[0], 0x12);

        nand.erase(0);
        assert!(nand.read(0).is_none());
        assert!(nand.read(1).is_none());
        assert_eq!(nand.read(4).unwrap()[0], 0x56);
        nand.program(0, Box::new([0x78; BLOCK_BYTES])).unwrap();
    }
    #[test]
    fn access_outside_the_device_is_rejected() {
        let geometry = NandGeometry::new(ERASE_BLOCKS, PAGES_PER_ERASE_BLOCK).unwrap();
        let mut nand = Nand::new(geometry).unwrap();
        let page = geometry.physical_pages();
        assert!(nand.read(page).is_none());
        let error = nand
            .program(page, Box::new([0x12; BLOCK_BYTES]))
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
}
