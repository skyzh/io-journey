use std::io;

use crate::ftl::LOGICAL_BLOCKS;
use crate::nand::BLOCK_BYTES;
use crate::queue::BlockQueue;
use crate::Trace;

const MAGIC: &[u8; 8] = b"IOJFS01\0";
const DIRECTORY_LBA: usize = 0;
const FIRST_DATA_LBA: usize = 1;
const LENGTH_FIELD: std::ops::Range<usize> = 8..12;
pub(crate) const FILE_CAPACITY: usize = (LOGICAL_BLOCKS - FIRST_DATA_LBA) * BLOCK_BYTES;

/// One root directory entry, /events.log, with a length and a fixed extent.
pub(crate) struct FileSystem<'a> {
    device: &'a mut BlockQueue,
    length: usize,
}

impl<'a> FileSystem<'a> {
    pub(crate) fn format(device: &'a mut BlockQueue, trace: &mut Trace) -> io::Result<Self> {
        let mut fs = Self { device, length: 0 };
        fs.write_length(0, trace)?;
        Ok(fs)
    }

    pub(crate) fn open(device: &'a mut BlockQueue, trace: &mut Trace) -> io::Result<Self> {
        let root = device.read(DIRECTORY_LBA, trace)?;
        if &root[..MAGIC.len()] != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "missing filesystem header",
            ));
        }
        let length = u32::from_le_bytes(root[LENGTH_FIELD].try_into().unwrap()) as usize;
        if length > FILE_CAPACITY {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "file length exceeds its extent",
            ));
        }
        trace.push(format!("fs     open /events.log at {length} bytes"));
        Ok(Self { device, length })
    }

    pub(crate) fn append(&mut self, mut bytes: &[u8], trace: &mut Trace) -> io::Result<()> {
        if bytes.len() > FILE_CAPACITY - self.length {
            return Err(io::Error::new(
                io::ErrorKind::StorageFull,
                "/events.log extent is full",
            ));
        }
        if bytes.is_empty() {
            return Ok(());
        }

        trace.push(format!(
            "fs     append {} bytes at offset {}",
            bytes.len(),
            self.length
        ));
        let mut offset = self.length;
        while !bytes.is_empty() {
            let lba = FIRST_DATA_LBA + offset / BLOCK_BYTES;
            let start = offset % BLOCK_BYTES;
            let count = bytes.len().min(BLOCK_BYTES - start);
            let mut block = if start == 0 {
                Box::new([0; BLOCK_BYTES])
            } else {
                self.device.read(lba, trace)?
            };
            block[start..start + count].copy_from_slice(&bytes[..count]);
            self.device.write(lba, block, trace)?;
            offset += count;
            bytes = &bytes[count..];
        }
        self.write_length(offset, trace)?;
        self.length = offset;
        Ok(())
    }

    pub(crate) fn read_all(&mut self, trace: &mut Trace) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::with_capacity(self.length);
        for index in 0..self.length.div_ceil(BLOCK_BYTES) {
            let block = self.device.read(FIRST_DATA_LBA + index, trace)?;
            let count = (self.length - bytes.len()).min(BLOCK_BYTES);
            bytes.extend_from_slice(&block[..count]);
        }
        Ok(bytes)
    }

    pub(crate) fn sync(&mut self, trace: &mut Trace) -> io::Result<()> {
        trace.push("fs     sync /events.log".into());
        self.device.flush(trace)
    }

    fn write_length(&mut self, length: usize, trace: &mut Trace) -> io::Result<()> {
        let mut root = Box::new([0; BLOCK_BYTES]);
        root[..MAGIC.len()].copy_from_slice(MAGIC);
        root[LENGTH_FIELD].copy_from_slice(&(length as u32).to_le_bytes());
        trace.push(format!("fs     queue directory length {length}"));
        self.device.write(DIRECTORY_LBA, root, trace)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ftl::Ftl;
    use std::num::NonZeroUsize;

    #[test]
    fn append_crosses_block_boundaries_and_capacity_failure_preserves_the_file() {
        let mut trace = Vec::new();
        let mut queue = BlockQueue::new(Ftl::new(), NonZeroUsize::MIN);
        let mut expected = vec![1; BLOCK_BYTES - 2];
        {
            let mut fs = FileSystem::format(&mut queue, &mut trace).unwrap();
            fs.append(&expected, &mut trace).unwrap();
            fs.append(&[2, 3, 4, 5], &mut trace).unwrap();
            expected.extend_from_slice(&[2, 3, 4, 5]);
            let events = trace.len();
            let err = fs.append(&vec![6; FILE_CAPACITY], &mut trace).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::StorageFull);
            assert_eq!(trace.len(), events);
            fs.sync(&mut trace).unwrap();
        }

        let mut queue = BlockQueue::new(Ftl::open(queue.power_cut()), NonZeroUsize::MIN);
        let mut fs = FileSystem::open(&mut queue, &mut trace).unwrap();
        assert_eq!(fs.read_all(&mut trace).unwrap(), expected);
    }
}
