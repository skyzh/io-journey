use std::io;
use std::ops::Range;

pub(crate) const BLOCK_BYTES: usize = 4096;
pub(crate) const PAGES_PER_ERASE_BLOCK: usize = 4;
pub(crate) const ERASE_BLOCKS: usize = 4;
pub(crate) const PHYSICAL_PAGES: usize = ERASE_BLOCKS * PAGES_PER_ERASE_BLOCK;
pub(crate) const ERASED_BYTE: u8 = 0xff;

pub(crate) type Block = Box<[u8; BLOCK_BYTES]>;

#[derive(Clone)]
pub(crate) struct Nand {
    pages: [Option<Block>; PHYSICAL_PAGES],
}

impl Nand {
    pub(crate) fn new() -> Self {
        Self {
            pages: std::array::from_fn(|_| None),
        }
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
        self.pages[erase_block_pages(erase_block)].fill(None);
    }
}

pub(crate) fn erase_block_pages(erase_block: usize) -> Range<usize> {
    let start = erase_block * PAGES_PER_ERASE_BLOCK;
    start..start + PAGES_PER_ERASE_BLOCK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn programming_requires_an_erased_page_and_erase_clears_the_whole_block() {
        let mut nand = Nand::new();
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
