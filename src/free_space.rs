use std::collections::BTreeSet;
use std::io;

use crate::nand::NandGeometry;

#[cfg_attr(feature = "learner-index", path = "../course/learner/index.rs")]
#[cfg_attr(not(feature = "learner-index"), path = "free_space/reference.rs")]
mod updates;
pub(crate) use updates::record_remap;

#[derive(Clone)]
pub(crate) struct FreePages {
    pages: BTreeSet<usize>,
    used_by_block: Vec<usize>,
}

impl FreePages {
    pub(crate) fn new(geometry: NandGeometry) -> io::Result<Self> {
        let used_by_block = empty_block_counts(geometry)?;
        Ok(Self {
            pages: (0..geometry.physical_pages()).collect(),
            used_by_block,
        })
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.pages.iter().copied()
    }

    pub(crate) fn count(&self) -> usize {
        self.pages.len()
    }

    pub(crate) fn used_in(&self, block: usize) -> usize {
        self.used_by_block[block]
    }
}

pub(crate) fn empty_block_counts(geometry: NandGeometry) -> io::Result<Vec<usize>> {
    let mut counts = Vec::new();
    counts
        .try_reserve_exact(geometry.erase_blocks())
        .map_err(io::Error::other)?;
    counts.resize(geometry.erase_blocks(), 0);
    Ok(counts)
}
