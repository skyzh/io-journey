use std::io;

use super::{Disk, WriteBack};

impl<D: Disk> WriteBack<D> {
    pub(super) fn evict_oldest(&mut self) -> io::Result<()> {
        let entry = self.entries.front().expect("full cache");
        if entry.dirty {
            self.disk.write_block(entry.lba, entry.block.clone())?;
        }
        self.entries.pop_front();
        Ok(())
    }

    pub(super) fn flush_dirty(&mut self) -> io::Result<()> {
        for entry in &self.entries {
            if entry.dirty {
                self.disk.write_block(entry.lba, entry.block.clone())?;
            }
        }
        self.disk.flush()?;
        for entry in &mut self.entries {
            entry.dirty = false;
        }
        Ok(())
    }
}
