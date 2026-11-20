use std::io;
use std::ops::Range;

pub const BLOCK_BYTES: usize = 4096;
pub(crate) const PAGES_PER_ERASE_BLOCK: usize = 4;
pub(crate) const ERASE_BLOCKS: usize = 4;
pub const ERASED_BYTE: u8 = 0xff;

pub type Block = Box<[u8; BLOCK_BYTES]>;

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
    pages: Vec<Option<Block>>,
}

impl Nand {
    pub(crate) fn new(geometry: NandGeometry) -> io::Result<Self> {
        let mut pages = Vec::new();
        pages
            .try_reserve_exact(geometry.physical_pages)
            .map_err(io::Error::other)?;
        pages.resize(geometry.physical_pages, None);
        Ok(Self { geometry, pages })
    }

    pub(crate) fn geometry(&self) -> NandGeometry {
        self.geometry
    }

    pub(crate) fn read(&self, page: usize) -> Option<&Block> {
        self.pages.get(page)?.as_ref()
    }

    pub(crate) fn erased_pages(&self) -> impl Iterator<Item = usize> + '_ {
        self.pages
            .iter()
            .enumerate()
            .filter_map(|(page, data)| data.is_none().then_some(page))
    }

    pub(crate) fn program(&mut self, page: usize, data: Block) -> io::Result<()> {
        let slot = self.pages.get_mut(page).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "physical page out of range")
        })?;
        if slot.is_some() {
            return Err(io::Error::other(
                "NAND page must be erased before programming",
            ));
        }
        *slot = Some(data);
        Ok(())
    }

    pub(crate) fn erase(&mut self, erase_block: usize) {
        self.pages[self.geometry.erase_block_pages(erase_block)].fill(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn programming_requires_an_erased_page_and_erase_clears_the_whole_block() {
        let geometry = NandGeometry::new(ERASE_BLOCKS, PAGES_PER_ERASE_BLOCK).unwrap();
        let mut nand = Nand::new(geometry).unwrap();
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
}
