use std::cell::RefCell;
use std::io;
use std::num::NonZeroUsize;

use io_journey::cache::WriteBack;
use io_journey::ftl::Ftl;
use io_journey::journal_fs::{Disk, JournalFs};
use io_journey::nand::{Block, NandGeometry, BLOCK_BYTES};
use io_journey::queue::{Operation, Response};
use io_journey::record;
use io_journey::ring::{commit_group, mock_write_wave, Request};
use io_journey::scheduler::{HostScheduler, Policy};

const RANDOM_FILE: &[u8] = b"random.bin";
const LOG_FILE: &[u8] = b"records.log";
const RANDOM_BLOCKS: u16 = 4;
const OPERATIONS: u8 = 8;
const LOG_BLOCKS: u16 = 3;
const ACKNOWLEDGED: usize = 2;

fn main() -> io::Result<()> {
    let ftl = Ftl::with_geometry(16, NandGeometry::new(8, 4)?)?;
    let mut filesystem = JournalFs::format(ScheduledDisk::new(ftl))?;
    let random = filesystem.create(RANDOM_FILE, 0o644)?;
    let log = filesystem.create(LOG_FILE, 0o644)?;
    filesystem.sync_dir()?;

    let expected_blocks = random_io(&mut filesystem, random)?;
    let expected_prefix = append_records(&mut filesystem, log)?;
    println!("Simulated power cut. Reopening the last durable image.");

    let image = filesystem.into_disk().scheduler.power_cut();
    let ftl = Ftl::open(image, &mut Vec::new())?;
    let mut recovered = JournalFs::open(ScheduledDisk::new(ftl))?;
    let random = recovered.lookup(RANDOM_FILE)?.ino;
    let log = recovered.lookup(LOG_FILE)?.ino;
    let mut random_file = JournalFile::new(&mut recovered, random, RANDOM_BLOCKS);
    for (index, expected) in expected_blocks.into_iter().enumerate() {
        verify_block(&mut random_file, index as u16, expected)?;
    }
    let bytes = recovered.read(log, 0, usize::from(LOG_BLOCKS) * BLOCK_BYTES)?;
    check(
        bytes == expected_prefix,
        "recovered log bytes differ from the acknowledged prefix",
    )?;
    let records = record::decode_acknowledged(&bytes, expected_prefix.len())?;
    check(
        records.len() == ACKNOWLEDGED,
        "recovered log has the wrong number of records",
    )?;
    println!(
        "Recovered all {ACKNOWLEDGED} acknowledged records ({} bytes). The unflushed tail is absent.",
        bytes.len()
    );
    println!("The synced random file also recovered byte for byte.");
    Ok(())
}

fn random_io(
    filesystem: &mut JournalFs<ScheduledDisk>,
    ino: u64,
) -> io::Result<[u8; RANDOM_BLOCKS as usize]> {
    let mut file = WriteBack::new(
        JournalFile::new(filesystem, ino, RANDOM_BLOCKS),
        NonZeroUsize::new(2).unwrap(),
    );
    let mut seed = 42_u32;
    let mut expected = [0_u8; RANDOM_BLOCKS as usize];
    for value in 1..=OPERATIONS {
        let lba = next_block(&mut seed);
        file.write_block(lba, Box::new([value; BLOCK_BYTES]))?;
        expected[usize::from(lba)] = value;
    }
    println!("Random writes: {OPERATIONS} aligned 4 KiB writes.");
    for _ in 0..OPERATIONS {
        let lba = next_block(&mut seed);
        verify_block(&mut file, lba, expected[usize::from(lba)])?;
    }
    println!("Random reads: {OPERATIONS} aligned 4 KiB reads matched.");
    for value in OPERATIONS + 1..=2 * OPERATIONS {
        let lba = next_block(&mut seed);
        file.write_block(lba, Box::new([value; BLOCK_BYTES]))?;
        expected[usize::from(lba)] = value;
        let lba = next_block(&mut seed);
        verify_block(&mut file, lba, expected[usize::from(lba)])?;
    }
    file.flush()?;
    println!("Mixed I/O: {OPERATIONS} aligned writes and {OPERATIONS} reads. The file is synced.");
    Ok(expected)
}

fn next_block(seed: &mut u32) -> u16 {
    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    (*seed >> 16) as u16 % RANDOM_BLOCKS
}

fn verify_block(file: &mut impl Disk, lba: u16, expected: u8) -> io::Result<()> {
    check(
        file.read_block(lba)?.iter().all(|&byte| byte == expected),
        &format!("unexpected data in random file block {lba}"),
    )
}

fn append_records(filesystem: &mut JournalFs<ScheduledDisk>, ino: u64) -> io::Result<Vec<u8>> {
    let payloads = [b'a', b'b', b'c'].map(|byte| vec![byte; BLOCK_BYTES - record::HEADER_BYTES]);
    let requests = payloads
        .iter()
        .enumerate()
        .map(|(index, payload)| {
            Ok(Request {
                id: index as u64 + 1,
                offset: (index * BLOCK_BYTES) as u64,
                bytes: record::encode(payload)?.into(),
            })
        })
        .collect::<io::Result<Vec<_>>>()?;
    let file = RefCell::new(WriteBack::new(
        JournalFile::new(filesystem, ino, LOG_BLOCKS),
        NonZeroUsize::MIN,
    ));
    let mut receipts = Vec::new();
    let (group, tail) = requests.split_at(ACKNOWLEDGED);
    commit_group(
        group,
        &mut |wave| {
            mock_write_wave(wave, |request| {
                write_record(&mut *file.borrow_mut(), request)
            })
        },
        &mut || file.borrow_mut().flush(),
        &mut receipts,
    )?;
    check(
        receipts.len() == ACKNOWLEDGED,
        "append group did not acknowledge every synced record",
    )?;
    let expected_prefix = group
        .iter()
        .flat_map(|request| request.bytes.iter().copied())
        .collect::<Vec<_>>();
    println!("Acknowledged {ACKNOWLEDGED} records after sync.");

    let mut file = file.into_inner();
    mock_write_wave(tail, |request| write_record(&mut file, request))?;
    // Reading the first block evicts the dirty tail from this one-block cache.
    // Eviction writes to the journal's live file without syncing it.
    file.read_block(0)?;
    file.into_inner();
    let live = filesystem.read(ino, 0, usize::from(LOG_BLOCKS) * BLOCK_BYTES)?;
    check(
        record::decode(&live)? == payloads,
        "the unflushed tail was not visible in the live journal file",
    )?;
    check(
        record::decode_acknowledged(&live, expected_prefix.len())? == payloads[..ACKNOWLEDGED],
        "the receipt did not select the acknowledged prefix from the live file",
    )?;
    println!("Read all {LOG_BLOCKS} records before the cut.");
    Ok(expected_prefix)
}

fn write_record(file: &mut impl Disk, request: &Request) -> io::Result<usize> {
    check(
        request.offset.is_multiple_of(BLOCK_BYTES as u64),
        "append request is not block aligned",
    )?;
    let lba = u16::try_from(request.offset / BLOCK_BYTES as u64)
        .map_err(|_| io::Error::other("append offset exceeds the file block range"))?;
    let block = request
        .bytes
        .as_ref()
        .try_into()
        .map_err(|_| io::Error::other("append request must contain one block"))?;
    file.write_block(lba, Box::new(block))?;
    Ok(BLOCK_BYTES)
}

fn check(condition: bool, message: &str) -> io::Result<()> {
    if condition {
        Ok(())
    } else {
        Err(io::Error::other(message))
    }
}

struct JournalFile<'a> {
    filesystem: &'a mut JournalFs<ScheduledDisk>,
    ino: u64,
    blocks: u16,
}

impl<'a> JournalFile<'a> {
    fn new(filesystem: &'a mut JournalFs<ScheduledDisk>, ino: u64, blocks: u16) -> Self {
        Self {
            filesystem,
            ino,
            blocks,
        }
    }

    fn offset(&self, lba: u16) -> io::Result<u64> {
        if lba >= self.blocks {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "file block out of range",
            ));
        }
        Ok(u64::from(lba) * BLOCK_BYTES as u64)
    }
}

impl Disk for JournalFile<'_> {
    fn block_count(&self) -> u16 {
        self.blocks
    }

    fn read_block(&mut self, lba: u16) -> io::Result<Block> {
        let offset = self.offset(lba)?;
        let bytes = self.filesystem.read(self.ino, offset, BLOCK_BYTES)?;
        let mut block = Box::new([0; BLOCK_BYTES]);
        block[..bytes.len()].copy_from_slice(&bytes);
        Ok(block)
    }

    fn write_block(&mut self, lba: u16, block: Block) -> io::Result<()> {
        let offset = self.offset(lba)?;
        let written = self.filesystem.write(self.ino, offset, block.as_ref())?;
        check(
            written == BLOCK_BYTES,
            "journal returned a short block write",
        )
    }

    fn flush(&mut self) -> io::Result<()> {
        self.filesystem.sync_file(self.ino)
    }
}

struct ScheduledDisk {
    scheduler: HostScheduler,
    writes: Vec<Operation>,
}

impl ScheduledDisk {
    fn new(ftl: Ftl) -> Self {
        Self {
            scheduler: HostScheduler::new(
                ftl,
                NonZeroUsize::new(2).unwrap(),
                Policy::BoundedReadFirst,
            ),
            writes: Vec::new(),
        }
    }

    fn complete(&mut self, operation: Operation) -> io::Result<Response> {
        // A read can finish ahead of a staged, disjoint write. Select its own
        // completion by ID, rather than assuming the last completion is ours.
        let id = self.writes.len() + 1;
        self.writes.push(operation);
        self.scheduler
            .execute(std::mem::take(&mut self.writes))?
            .into_iter()
            .find(|finished| finished.id == id)
            .map(|finished| finished.response)
            .ok_or_else(|| io::Error::other("missing scheduled disk completion"))
    }
}

impl Disk for ScheduledDisk {
    fn block_count(&self) -> u16 {
        u16::try_from(self.scheduler.logical_blocks()).expect("demo disk capacity fits u16")
    }

    fn read_block(&mut self, lba: u16) -> io::Result<Block> {
        match self.complete(Operation::Read {
            lba: usize::from(lba),
            blocks: 1,
        })? {
            Response::Read(mut blocks) if blocks.len() == 1 => Ok(blocks.remove(0)),
            _ => Err(io::Error::other(
                "scheduled read returned an unexpected response",
            )),
        }
    }

    fn write_block(&mut self, lba: u16, block: Block) -> io::Result<()> {
        // Leave a few adjacent requests for the host scheduler to admit.
        self.writes.push(Operation::Write {
            lba: usize::from(lba),
            data: vec![block],
        });
        if self.writes.len() == 4 {
            self.scheduler.execute(std::mem::take(&mut self.writes))?;
        }
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        check(
            matches!(self.complete(Operation::Flush)?, Response::Done),
            "scheduled flush returned an unexpected response",
        )
    }
}
