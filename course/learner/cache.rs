use std::io;

use super::{Disk, WriteBack};

impl<D: Disk> WriteBack<D> {
    pub(super) fn evict_oldest(&mut self) -> io::Result<()> {
        Err(io::Error::other("implement dirty eviction"))
    }

    pub(super) fn flush_dirty(&mut self) -> io::Result<()> {
        Err(io::Error::other("implement dirty flush"))
    }
}
