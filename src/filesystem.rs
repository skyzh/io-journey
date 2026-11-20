use std::io;

use crate::nand::BLOCK_BYTES;
use crate::queue::BlockQueue;
use crate::Trace;

const MAGIC: &[u8; 8] = b"IOJFS01\0";
const DIRECTORY_LBA: usize = 0;
pub(crate) const FIRST_DATA_LBA: usize = 1;
const LENGTH_FIELD: std::ops::Range<usize> = 8..12;

/// One root directory entry, /events.log, with a length and a fixed extent.
pub(crate) struct FileSystem<'a> {
    device: &'a mut BlockQueue,
    length: usize,
    capacity: usize,
}

impl<'a> FileSystem<'a> {
    pub(crate) fn format(device: &'a mut BlockQueue, trace: &mut Trace) -> io::Result<Self> {
        let capacity = file_capacity(device)?;
        let mut fs = Self {
            device,
            length: 0,
            capacity,
        };
        fs.write_length(0, trace)?;
        Ok(fs)
    }

    pub(crate) fn open(device: &'a mut BlockQueue, trace: &mut Trace) -> io::Result<Self> {
        let capacity = file_capacity(device)?;
        let root = device.read(DIRECTORY_LBA, trace)?;
        if &root[..MAGIC.len()] != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "missing filesystem header",
            ));
        }
        let length = u32::from_le_bytes(root[LENGTH_FIELD].try_into().unwrap()) as usize;
        if length > capacity {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "file length exceeds its extent",
            ));
        }
        trace.push(format!("fs     open /events.log at {length} bytes"));
        Ok(Self {
            device,
            length,
            capacity,
        })
    }

    pub(crate) fn capacity(&self) -> usize {
        self.capacity
    }

    pub(crate) fn append(&mut self, mut bytes: &[u8], trace: &mut Trace) -> io::Result<()> {
        if bytes.len() > self.capacity - self.length {
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
        let first_lba = FIRST_DATA_LBA + self.length / BLOCK_BYTES;
        let mut blocks = Vec::new();
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
            blocks.push(block);
            offset += count;
            bytes = &bytes[count..];
        }
        self.device.write_range(first_lba, blocks, trace)?;
        self.write_length(offset, trace)?;
        self.length = offset;
        Ok(())
    }

    pub(crate) fn read_all(&mut self, trace: &mut Trace) -> io::Result<Vec<u8>> {
        if self.length == 0 {
            return Ok(Vec::new());
        }
        let count = self.length.div_ceil(BLOCK_BYTES);
        let mut bytes = Vec::with_capacity(self.length);
        for block in self.device.read_range(FIRST_DATA_LBA, count, trace)? {
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

fn file_capacity(device: &BlockQueue) -> io::Result<usize> {
    let max_data_blocks = u32::MAX as usize / BLOCK_BYTES;
    let data_blocks = device.logical_blocks() - FIRST_DATA_LBA;
    if !(1..=max_data_blocks).contains(&data_blocks) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "filesystem needs a nonempty data extent that fits its length field",
        ));
    }
    Ok(data_blocks * BLOCK_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app;
    use crate::ftl::{Ftl, TornProgram};
    use crate::nand::NandGeometry;
    use std::num::NonZeroUsize;

    #[test]
    fn append_crosses_block_boundaries_and_capacity_failure_preserves_the_file() {
        let mut trace = Vec::new();
        let geometry = NandGeometry::new(4, 4).unwrap();
        let ftl = Ftl::with_geometry(10, geometry).unwrap();
        let mut queue = BlockQueue::new(ftl, NonZeroUsize::MIN);
        let mut expected = vec![1; BLOCK_BYTES - 2];
        {
            let mut fs = FileSystem::format(&mut queue, &mut trace).unwrap();
            fs.append(&expected, &mut trace).unwrap();
            fs.append(&[2, 3, 4, 5], &mut trace).unwrap();
            expected.extend_from_slice(&[2, 3, 4, 5]);
            let events = trace.len();
            assert_eq!(fs.capacity(), 9 * BLOCK_BYTES);
            let err = fs.append(&vec![6; fs.capacity()], &mut trace).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::StorageFull);
            assert_eq!(trace.len(), events);
            fs.sync(&mut trace).unwrap();
        }

        let ftl = Ftl::open(queue.power_cut(), &mut trace).unwrap();
        let mut queue = BlockQueue::new(ftl, NonZeroUsize::MIN);
        let mut fs = FileSystem::open(&mut queue, &mut trace).unwrap();
        assert_eq!(fs.read_all(&mut trace).unwrap(), expected);
    }

    #[test]
    fn sync_reports_a_later_child_failure_and_reopen_keeps_the_committed_record() {
        let mut trace = Vec::new();
        let depth = NonZeroUsize::new(2).unwrap();
        let mut queue = BlockQueue::new(Ftl::new(), depth);
        {
            let mut fs = FileSystem::format(&mut queue, &mut trace).unwrap();
            app::append(&mut fs, b"committed", &mut trace).unwrap();
            fs.sync(&mut trace).unwrap();
        }
        let second_lba = FIRST_DATA_LBA + 1;
        queue.arm_torn_write(second_lba, 37).unwrap();
        {
            let mut fs = FileSystem::open(&mut queue, &mut trace).unwrap();
            app::append(&mut fs, &[0x45; BLOCK_BYTES], &mut trace).unwrap();
            let error = fs.sync(&mut trace).unwrap_err();
            let torn = error
                .get_ref()
                .unwrap()
                .downcast_ref::<TornProgram>()
                .unwrap();
            assert_eq!(torn.lba, second_lba);
        }

        let ftl = Ftl::open(queue.power_cut(), &mut trace).unwrap();
        let mut queue = BlockQueue::new(ftl, depth);
        let mut fs = FileSystem::open(&mut queue, &mut trace).unwrap();
        assert_eq!(
            app::recover(&mut fs, &mut trace).unwrap(),
            [b"committed".to_vec()]
        );
    }
}
