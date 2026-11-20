//! A small root directory with copy-on-write file data and a metadata redo log.

mod format;
#[cfg_attr(feature = "learner-journal", path = "../course/learner/journal.rs")]
#[cfg_attr(not(feature = "learner-journal"), path = "journal_fs/reference.rs")]
mod protocol;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::io;

use self::format::{commit_record, frame_checksum, Metadata};
use crate::nand::{Block, BLOCK_BYTES};

pub const ROOT_INODE: u64 = 1;
pub const MAX_FILES: usize = 12;
pub const MAX_NAME_BYTES: usize = 48;
const FILE_BLOCKS: usize = 128;
pub const MAX_FILE_BYTES: usize = FILE_BLOCKS * BLOCK_BYTES;

const HOME: u16 = 0;
const REDO: u16 = 1;
const COMMIT: u16 = 2;
const FIRST_DATA: u16 = 3;

/// An aligned logical block device. A successful flush covers all earlier writes.
pub trait Disk {
    fn block_count(&self) -> u16;
    fn read_block(&mut self, lba: u16) -> io::Result<Block>;
    fn write_block(&mut self, lba: u16, block: Block) -> io::Result<()>;
    fn flush(&mut self) -> io::Result<()>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileInfo {
    pub ino: u64,
    pub len: usize,
    pub mode: u16,
    pub blocks: usize,
    pub linked: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Node {
    ino: u64,
    len: usize,
    mode: u16,
    blocks: [u16; FILE_BLOCKS],
}

impl Node {
    fn info(&self, linked: bool) -> FileInfo {
        FileInfo {
            ino: self.ino,
            len: self.len,
            mode: self.mode,
            blocks: self.blocks.iter().filter(|&&lba| lba != 0).count(),
            linked,
        }
    }
}

pub struct JournalFs<D> {
    disk: D,
    live: Metadata,
    durable: Metadata,
    handles: BTreeMap<u64, usize>,
    detached: BTreeMap<u64, Node>,
    needs_reopen: bool,
}

impl<D: Disk> JournalFs<D> {
    pub fn format(mut disk: D) -> io::Result<Self> {
        check_geometry(&disk)?;
        let metadata = Metadata::empty();
        disk.write_block(HOME, metadata.encode())?;
        disk.write_block(COMMIT, Box::new([0; BLOCK_BYTES]))?;
        disk.flush()?;
        Ok(Self::new(disk, metadata))
    }

    pub fn open(mut disk: D) -> io::Result<Self> {
        check_geometry(&disk)?;
        let metadata = recover_metadata(&mut disk)?;
        Ok(Self::new(disk, metadata))
    }

    fn new(disk: D, metadata: Metadata) -> Self {
        Self {
            disk,
            live: metadata.clone(),
            durable: metadata,
            handles: BTreeMap::new(),
            detached: BTreeMap::new(),
            needs_reopen: false,
        }
    }

    pub fn into_disk(self) -> D {
        self.disk
    }

    pub fn generation(&self) -> u64 {
        self.durable.generation
    }

    pub fn data_blocks(&self) -> usize {
        usize::from(self.disk.block_count() - FIRST_DATA)
    }

    pub fn free_blocks(&self) -> usize {
        self.data_blocks() - self.allocated().len()
    }

    pub fn lookup(&self, name: &[u8]) -> io::Result<FileInfo> {
        self.ready()?;
        self.live
            .files
            .get(name)
            .map(|node| node.info(true))
            .ok_or_else(missing)
    }

    pub fn stat(&self, ino: u64) -> io::Result<FileInfo> {
        self.ready()?;
        if let Some(node) = self.live.files.values().find(|node| node.ino == ino) {
            Ok(node.info(true))
        } else {
            self.detached
                .get(&ino)
                .map(|node| node.info(false))
                .ok_or_else(missing)
        }
    }

    pub fn entries(&self) -> io::Result<impl Iterator<Item = (&[u8], FileInfo)>> {
        self.ready()?;
        Ok(self
            .live
            .files
            .iter()
            .map(|(name, node)| (name.as_slice(), node.info(true))))
    }

    pub fn create(&mut self, name: &[u8], mode: u16) -> io::Result<u64> {
        self.ready()?;
        check_name(name)?;
        if self.live.files.contains_key(name) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "name already exists",
            ));
        }
        if self.live.files.len() == MAX_FILES {
            return Err(full());
        }
        let ino = self.live.next_inode;
        self.live.next_inode = ino.checked_add(1).ok_or_else(full)?;
        self.live.files.insert(
            name.to_vec(),
            Node {
                ino,
                len: 0,
                mode: mode & 0o777,
                blocks: [0; FILE_BLOCKS],
            },
        );
        Ok(ino)
    }

    pub fn retain_file(&mut self, ino: u64) -> io::Result<()> {
        self.stat(ino)?;
        *self.handles.entry(ino).or_default() += 1;
        Ok(())
    }

    pub fn release_file(&mut self, ino: u64) {
        if let Some(count) = self.handles.get_mut(&ino) {
            *count -= 1;
            if *count == 0 {
                self.handles.remove(&ino);
                self.detached.remove(&ino);
            }
        }
    }

    pub fn unlink(&mut self, name: &[u8]) -> io::Result<()> {
        self.ready()?;
        let node = self.live.files.remove(name).ok_or_else(missing)?;
        self.detach(node);
        Ok(())
    }

    pub fn rename(&mut self, from: &[u8], to: &[u8], replace: bool) -> io::Result<()> {
        self.ready()?;
        check_name(to)?;
        self.lookup(from)?;
        if !replace && self.live.files.contains_key(to) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "destination exists",
            ));
        }
        if from == to {
            return Ok(());
        }
        let node = self.live.files.remove(from).expect("source present");
        if let Some(old) = self.live.files.insert(to.to_vec(), node) {
            self.detach(old);
        }
        Ok(())
    }

    pub fn read(&mut self, ino: u64, offset: u64, size: usize) -> io::Result<Vec<u8>> {
        self.ready()?;
        let node = self.node(ino)?.clone();
        if offset >= node.len as u64 || size == 0 {
            return Ok(Vec::new());
        }
        let mut offset = offset as usize;
        let count = size.min(node.len - offset);
        let mut data = Vec::with_capacity(count);
        let end = offset + count;
        while offset < end {
            let within = offset % BLOCK_BYTES;
            let count = (end - offset).min(BLOCK_BYTES - within);
            let block = self.read_data(node.blocks[offset / BLOCK_BYTES])?;
            data.extend_from_slice(&block[within..within + count]);
            offset += count;
        }
        Ok(data)
    }

    pub fn write(&mut self, ino: u64, offset: u64, bytes: &[u8]) -> io::Result<usize> {
        self.ready()?;
        let mut node = self.node(ino)?.clone();
        if bytes.is_empty() {
            return Ok(0);
        }
        let offset = bounded_size(offset)?;
        let end = offset
            .checked_add(bytes.len())
            .filter(|&end| end <= MAX_FILE_BYTES)
            .ok_or_else(too_large)?;
        let first = offset / BLOCK_BYTES;
        let destinations = self.reserve(end.div_ceil(BLOCK_BYTES) - first)?;
        let mut offset = offset;
        let mut remaining = bytes;
        for (index, destination) in (first..).zip(destinations) {
            let within = offset % BLOCK_BYTES;
            let count = remaining.len().min(BLOCK_BYTES - within);
            let mut block = if count == BLOCK_BYTES {
                Box::new([0; BLOCK_BYTES])
            } else {
                self.read_data(node.blocks[index])?
            };
            block[within..within + count].copy_from_slice(&remaining[..count]);
            self.disk.write_block(destination, block)?;
            node.blocks[index] = destination;
            remaining = &remaining[count..];
            offset += count;
        }
        node.len = node.len.max(end);
        *self.node_mut(ino)? = node;
        Ok(bytes.len())
    }

    pub fn truncate(&mut self, ino: u64, len: u64) -> io::Result<()> {
        self.ready()?;
        let len = bounded_size(len)?;
        let mut node = self.node(ino)?.clone();
        if len < node.len {
            node.blocks[len.div_ceil(BLOCK_BYTES)..].fill(0);
            let start = len % BLOCK_BYTES;
            let index = len / BLOCK_BYTES;
            if start != 0 && node.blocks[index] != 0 {
                let destination = self.reserve(1)?[0];
                let mut block = self.read_data(node.blocks[index])?;
                block[start..].fill(0);
                self.disk.write_block(destination, block)?;
                node.blocks[index] = destination;
            }
        }
        node.len = len;
        *self.node_mut(ino)? = node;
        Ok(())
    }

    /// Sync data and length under the inode's already durable name, if it has one.
    pub fn sync_file(&mut self, ino: u64) -> io::Result<()> {
        self.ready()?;
        let node = self.node(ino)?.clone();
        let mut next = self.durable.clone();
        if let Some(entry) = next.files.values_mut().find(|entry| entry.ino == ino) {
            *entry = node;
        }
        self.commit(next)
    }

    /// Publish the current root names and their contents in a single transaction.
    pub fn sync_dir(&mut self) -> io::Result<()> {
        self.ready()?;
        self.commit(self.live.clone())
    }

    fn commit(&mut self, mut next: Metadata) -> io::Result<()> {
        if next.files == self.durable.files && next.next_inode == self.durable.next_inode {
            return self.disk.flush();
        }
        next.generation = self.durable.generation.checked_add(1).ok_or_else(full)?;
        let redo = next.encode();
        let marker = commit_record(next.generation, frame_checksum(&redo));

        // A failed barrier can have reached the device. Block further updates
        // until replay determines which allocations the durable metadata owns.
        self.needs_reopen = true;
        protocol::persist_redo(&mut self.disk, &redo, marker)?;
        install_home(&mut self.disk, redo)?;
        self.durable = next;
        self.needs_reopen = false;
        Ok(())
    }

    fn ready(&self) -> io::Result<()> {
        if self.needs_reopen {
            Err(io::Error::other("journal update failed, reopen required"))
        } else {
            Ok(())
        }
    }

    fn node(&self, ino: u64) -> io::Result<&Node> {
        self.live
            .files
            .values()
            .find(|node| node.ino == ino)
            .or_else(|| self.detached.get(&ino))
            .ok_or_else(missing)
    }

    fn node_mut(&mut self, ino: u64) -> io::Result<&mut Node> {
        if let Some(node) = self.live.files.values_mut().find(|node| node.ino == ino) {
            Ok(node)
        } else {
            self.detached.get_mut(&ino).ok_or_else(missing)
        }
    }

    fn detach(&mut self, node: Node) {
        if self.handles.contains_key(&node.ino) {
            self.detached.insert(node.ino, node);
        }
    }

    fn read_data(&mut self, lba: u16) -> io::Result<Block> {
        if lba == 0 {
            Ok(Box::new([0; BLOCK_BYTES]))
        } else {
            self.disk.read_block(lba)
        }
    }

    fn allocated(&self) -> BTreeSet<u16> {
        self.live
            .files
            .values()
            .chain(self.durable.files.values())
            .chain(self.detached.values())
            .flat_map(|node| node.blocks.iter().copied())
            .filter(|&lba| lba != 0)
            .collect()
    }

    fn reserve(&self, count: usize) -> io::Result<Vec<u16>> {
        let allocated = self.allocated();
        let free: Vec<_> = (FIRST_DATA..self.disk.block_count())
            .filter(|lba| !allocated.contains(lba))
            .take(count)
            .collect();
        if free.len() != count {
            return Err(full());
        }
        Ok(free)
    }
}

fn install_home(disk: &mut impl Disk, redo: Block) -> io::Result<()> {
    disk.write_block(HOME, redo)?;
    disk.flush()?;
    disk.write_block(COMMIT, Box::new([0; BLOCK_BYTES]))?;
    disk.flush()
}

fn recover_metadata(disk: &mut impl Disk) -> io::Result<Metadata> {
    let (metadata, redo) = protocol::read_committed(disk)?;
    if let Some(redo) = redo {
        install_home(disk, redo)?;
    }
    Ok(metadata)
}

fn check_geometry(disk: &impl Disk) -> io::Result<()> {
    if disk.block_count() <= FIRST_DATA {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "need three metadata blocks and data space",
        ));
    }
    Ok(())
}

fn check_name(name: &[u8]) -> io::Result<()> {
    if name.is_empty()
        || name.len() > MAX_NAME_BYTES
        || name == b"."
        || name == b".."
        || name.iter().any(|byte| matches!(byte, 0 | b'/'))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid root file name",
        ));
    }
    Ok(())
}

fn bounded_size(size: u64) -> io::Result<usize> {
    if size > MAX_FILE_BYTES as u64 {
        return Err(too_large());
    }
    Ok(size as usize)
}

fn missing() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "file does not exist")
}

fn full() -> io::Error {
    io::Error::new(
        io::ErrorKind::StorageFull,
        "filesystem has no space for the update",
    )
}

fn too_large() -> io::Error {
    io::Error::new(
        io::ErrorKind::FileTooLarge,
        "file exceeds the journal format limit",
    )
}

fn corrupt(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
