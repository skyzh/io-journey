use std::io;
use std::ops::Range;

#[derive(Clone)]
pub(super) struct Mapping {
    entries: Vec<Option<usize>>,
}

impl Mapping {
    pub(super) fn new(logical_blocks: usize) -> io::Result<Self> {
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(logical_blocks)
            .map_err(io::Error::other)?;
        entries.resize(logical_blocks, None);
        Ok(Self { entries })
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn location(&self, lba: usize) -> Option<usize> {
        self.entries.get(lba).copied().flatten()
    }

    #[cfg_attr(feature = "learner", allow(dead_code))]
    pub(super) fn replace(&mut self, lba: usize, page: usize) -> Option<usize> {
        self.entries[lba].replace(page)
    }

    pub(super) fn live_in(&self, pages: &Range<usize>) -> usize {
        self.entries
            .iter()
            .flatten()
            .filter(|page| pages.contains(page))
            .count()
    }
}

impl std::ops::Deref for Mapping {
    type Target = [Option<usize>];

    fn deref(&self) -> &Self::Target {
        &self.entries
    }
}

impl std::ops::DerefMut for Mapping {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.entries
    }
}

#[cfg(test)]
#[test]
fn scan_mapping_accepts_the_original_learner_edits() {
    let mut map = Mapping::new(2).unwrap();
    assert_eq!(map[0].replace(1), None);
    for page in map.iter_mut().flatten() {
        *page = 2;
    }
    assert_eq!(map.location(0), Some(2));
    assert_eq!(map.live_in(&(0..2)), 0);
    assert_eq!(map.live_in(&(2..4)), 1);
}
