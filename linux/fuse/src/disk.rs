use std::fs::{File, OpenOptions};
use std::io::{self, Seek, SeekFrom};
use std::os::unix::fs::{FileExt, FileTypeExt, OpenOptionsExt};
use std::path::Path;

use io_journey::journal_fs::Disk;
use io_journey::nand::{Block, BLOCK_BYTES};

use crate::invalid;

const IMAGE_BYTES: u64 = 8 * 1024 * 1024;

#[repr(align(4096))]
struct Aligned([u8; BLOCK_BYTES]);

pub struct FileDisk {
    file: File,
    blocks: u16,
}

impl FileDisk {
    pub fn new_image(path: &Path) -> io::Result<Self> {
        let file = options().create_new(true).mode(0o600).open(path)?;
        file.set_len(IMAGE_BYTES)?;
        Self::with_len(file, IMAGE_BYTES)
    }

    pub fn open_device(path: &Path) -> io::Result<Self> {
        let mut file = options().open(path)?;
        if !file.metadata()?.file_type().is_block_device() {
            return Err(invalid("format-device expects the owned lab block device"));
        }
        let len = file.seek(SeekFrom::End(0))?;
        Self::with_len(file, len)
    }

    pub fn open(path: &Path) -> io::Result<Self> {
        let mut file = options().open(path)?;
        let metadata = file.metadata()?;
        let len = if metadata.is_file() {
            metadata.len()
        } else if metadata.file_type().is_block_device() {
            file.seek(SeekFrom::End(0))?
        } else {
            return Err(invalid("expected a regular image or block device"));
        };
        Self::with_len(file, len)
    }

    fn with_len(file: File, len: u64) -> io::Result<Self> {
        let bytes = BLOCK_BYTES as u64;
        if len % bytes != 0 || len / bytes > u64::from(u16::MAX) {
            return Err(invalid("backend must fit the 4 KiB block address space"));
        }
        Ok(Self {
            file,
            blocks: (len / bytes) as u16,
        })
    }
}

impl Disk for FileDisk {
    fn block_count(&self) -> u16 {
        self.blocks
    }

    fn read_block(&mut self, lba: u16) -> io::Result<Block> {
        let mut buffer = Aligned([0; BLOCK_BYTES]);
        self.file
            .read_exact_at(&mut buffer.0, u64::from(lba) * BLOCK_BYTES as u64)?;
        Ok(Box::new(buffer.0))
    }

    fn write_block(&mut self, lba: u16, block: Block) -> io::Result<()> {
        let buffer = Aligned(*block);
        self.file
            .write_all_at(&buffer.0, u64::from(lba) * BLOCK_BYTES as u64)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.sync_all()
    }
}

fn options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .custom_flags(libc::O_DIRECT | libc::O_NOFOLLOW);
    options
}
