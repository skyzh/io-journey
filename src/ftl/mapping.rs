use std::io;
use std::ops::Range;

use crate::nand::NandGeometry;

#[derive(Clone)]
pub(super) struct Mapping {
    entries: Vec<Option<usize>>,
    #[cfg(feature = "indexed-ftl")]
    live_by_block: Vec<usize>,
    #[cfg(feature = "indexed-ftl")]
    pages_per_block: usize,
}

impl Mapping {
    pub(super) fn new(logical_blocks: usize, _geometry: NandGeometry) -> io::Result<Self> {
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(logical_blocks)
            .map_err(io::Error::other)?;
        entries.resize(logical_blocks, None);
        Ok(Self {
            entries,
            #[cfg(feature = "indexed-ftl")]
            live_by_block: crate::free_space::empty_block_counts(_geometry)?,
            #[cfg(feature = "indexed-ftl")]
            pages_per_block: _geometry.pages_per_erase_block(),
        })
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn location(&self, lba: usize) -> Option<usize> {
        self.entries.get(lba).copied().flatten()
    }

    #[cfg_attr(
        all(feature = "learner", not(feature = "indexed-ftl")),
        allow(dead_code)
    )]
    pub(super) fn replace(&mut self, lba: usize, page: usize) -> Option<usize> {
        let previous = self.entries[lba].replace(page);
        #[cfg(feature = "indexed-ftl")]
        crate::free_space::record_remap(
            &mut self.live_by_block,
            self.pages_per_block,
            previous,
            page,
        );
        previous
    }

    #[cfg(not(feature = "indexed-ftl"))]
    pub(super) fn live_in(&self, _block: usize, pages: &Range<usize>) -> usize {
        self.entries
            .iter()
            .flatten()
            .filter(|page| pages.contains(page))
            .count()
    }

    #[cfg(feature = "indexed-ftl")]
    pub(super) fn live_in(&self, block: usize, _pages: &Range<usize>) -> usize {
        self.live_by_block[block]
    }
}

#[cfg(feature = "indexed-ftl")]
impl std::ops::Index<usize> for Mapping {
    type Output = Option<usize>;

    fn index(&self, index: usize) -> &Self::Output {
        &self.entries[index]
    }
}

// The scan path accepts the first lesson's original mutable map edits.
// Indexed builds use replace to update the map and its counts together.
#[cfg(not(feature = "indexed-ftl"))]
impl std::ops::Deref for Mapping {
    type Target = [Option<usize>];

    fn deref(&self) -> &Self::Target {
        &self.entries
    }
}

#[cfg(not(feature = "indexed-ftl"))]
impl std::ops::DerefMut for Mapping {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.entries
    }
}

#[cfg(all(test, not(feature = "indexed-ftl")))]
#[test]
fn scan_mapping_accepts_the_original_learner_edits() {
    let mut map = Mapping::new(2, NandGeometry::new(2, 2).unwrap()).unwrap();
    assert_eq!(map[0].replace(1), None);
    for page in map.iter_mut().flatten() {
        *page = 2;
    }
    assert_eq!(map.location(0), Some(2));
    assert_eq!(map.live_in(0, &(0..2)), 0);
    assert_eq!(map.live_in(1, &(2..4)), 1);
}
