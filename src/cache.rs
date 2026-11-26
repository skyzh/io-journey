use std::collections::VecDeque;
use std::io;
use std::num::NonZeroUsize;

use crate::journal_fs::Disk;
use crate::nand::Block;

#[cfg_attr(feature = "learner-cache", path = "../course/learner/cache.rs")]
#[cfg_attr(not(feature = "learner-cache"), path = "cache/reference.rs")]
mod write_back;

struct Entry {
    lba: u16,
    block: Block,
    dirty: bool,
}

/// A bounded FIFO write-back cache in front of an aligned block interface.
pub struct WriteBack<D> {
    disk: D,
    capacity: NonZeroUsize,
    entries: VecDeque<Entry>,
}

impl<D: Disk> WriteBack<D> {
    pub fn new(disk: D, capacity: NonZeroUsize) -> Self {
        Self {
            disk,
            capacity,
            entries: VecDeque::new(),
        }
    }

    /// Discard resident cache entries and return the backing disk without syncing.
    pub fn into_inner(self) -> D {
        self.disk
    }

    fn check_lba(&self, lba: u16) -> io::Result<()> {
        if lba >= self.disk.block_count() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cache LBA out of range",
            ));
        }
        Ok(())
    }

    fn insert(&mut self, entry: Entry) -> io::Result<()> {
        if self.entries.len() == self.capacity.get() {
            self.evict_oldest()?;
        }
        self.entries.push_back(entry);
        Ok(())
    }
}

impl<D: Disk> Disk for WriteBack<D> {
    fn block_count(&self) -> u16 {
        self.disk.block_count()
    }

    fn read_block(&mut self, lba: u16) -> io::Result<Block> {
        self.check_lba(lba)?;
        if let Some(entry) = self.entries.iter().find(|entry| entry.lba == lba) {
            return Ok(entry.block.clone());
        }
        let block = self.disk.read_block(lba)?;
        self.insert(Entry {
            lba,
            block: block.clone(),
            dirty: false,
        })?;
        Ok(block)
    }

    fn write_block(&mut self, lba: u16, block: Block) -> io::Result<()> {
        self.check_lba(lba)?;
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.lba == lba) {
            entry.block = block;
            entry.dirty = true;
            return Ok(());
        }
        self.insert(Entry {
            lba,
            block,
            dirty: true,
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flush_dirty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ftl::Ftl;
    use crate::nand::BLOCK_BYTES;
    use crate::queue::BlockQueue;
    use crate::Trace;

    struct ModelDisk {
        queue: BlockQueue,
        trace: Trace,
        writes: Vec<u16>,
        fail_write: bool,
        fail_flush: bool,
    }

    impl ModelDisk {
        fn new(ftl: Ftl) -> Self {
            Self {
                queue: BlockQueue::new(ftl, NonZeroUsize::MIN),
                trace: Trace::new(),
                writes: Vec::new(),
                fail_write: false,
                fail_flush: false,
            }
        }

        fn cut(self) -> Self {
            Self::new(Ftl::open(self.queue.power_cut(), &mut Trace::new()).unwrap())
        }
    }

    impl Disk for ModelDisk {
        fn block_count(&self) -> u16 {
            self.queue.logical_blocks().try_into().unwrap()
        }

        fn read_block(&mut self, lba: u16) -> io::Result<Block> {
            self.queue.read(usize::from(lba), &mut self.trace)
        }

        fn write_block(&mut self, lba: u16, block: Block) -> io::Result<()> {
            if self.fail_write {
                return Err(io::Error::other("injected write failure"));
            }
            self.queue.write(usize::from(lba), block, &mut self.trace)?;
            self.writes.push(lba);
            Ok(())
        }

        fn flush(&mut self) -> io::Result<()> {
            if self.fail_flush {
                return Err(io::Error::other("injected flush failure"));
            }
            self.queue.flush(&mut self.trace)
        }
    }

    fn block(byte: u8) -> Block {
        Box::new([byte; BLOCK_BYTES])
    }

    #[test]
    fn dirty_reads_are_visible_before_sync_and_only_synced_data_recovers() {
        let disk = ModelDisk::new(Ftl::new());
        let mut cache = WriteBack::new(disk, NonZeroUsize::MIN);
        cache.write_block(0, block(0x12)).unwrap();
        assert_eq!(cache.read_block(0).unwrap(), block(0x12));
        assert!(cache.disk.writes.is_empty());
        let mut disk = cache.into_inner().cut();
        assert_eq!(disk.read_block(0).unwrap(), block(0));

        let mut cache = WriteBack::new(disk, NonZeroUsize::MIN);
        cache.write_block(0, block(0x12)).unwrap();
        cache.flush().unwrap();
        let mut disk = cache.into_inner().cut();
        assert_eq!(disk.read_block(0).unwrap(), block(0x12));
    }

    #[test]
    fn eviction_writes_back_without_syncing_and_clean_eviction_needs_no_write() {
        let mut cache = WriteBack::new(ModelDisk::new(Ftl::new()), NonZeroUsize::MIN);
        cache.write_block(0, block(0x12)).unwrap();
        assert_eq!(cache.read_block(1).unwrap(), block(0));
        assert_eq!(
            cache.disk.writes,
            [0],
            "evicting a dirty block issues a write"
        );
        assert_eq!(cache.read_block(2).unwrap(), block(0));
        assert_eq!(
            cache.disk.writes,
            [0],
            "evicting a clean block does not write"
        );
        let mut disk = cache.into_inner().cut();
        assert_eq!(
            disk.read_block(0).unwrap(),
            block(0),
            "eviction is not a flush"
        );

        let mut cache = WriteBack::new(disk, NonZeroUsize::MIN);
        cache.write_block(0, block(0x12)).unwrap();
        cache.read_block(1).unwrap();
        cache.flush().unwrap();
        let mut disk = cache.into_inner().cut();
        assert_eq!(
            disk.read_block(0).unwrap(),
            block(0x12),
            "flush includes evicted writes"
        );
    }

    #[test]
    fn failed_eviction_and_flush_keep_dirty_entries_for_retry() {
        let mut cache = WriteBack::new(ModelDisk::new(Ftl::new()), NonZeroUsize::MIN);
        cache.write_block(0, block(0x12)).unwrap();
        cache.disk.fail_write = true;
        assert_eq!(
            cache.write_block(1, block(0x34)).unwrap_err().to_string(),
            "injected write failure"
        );
        assert_eq!(cache.read_block(0).unwrap(), block(0x12));
        cache.disk.fail_write = false;
        cache.disk.fail_flush = true;
        assert_eq!(
            cache.flush().unwrap_err().to_string(),
            "injected flush failure"
        );
        assert!(cache.entries[0].dirty);
        cache.disk.fail_flush = false;
        cache.flush().unwrap();
        assert!(!cache.entries[0].dirty);
        let mut disk = cache.into_inner().cut();
        assert_eq!(disk.read_block(0).unwrap(), block(0x12));
        assert_eq!(disk.read_block(1).unwrap(), block(0));
    }
}
