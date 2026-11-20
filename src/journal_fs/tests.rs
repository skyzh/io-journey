use super::*;
use crate::ftl::Ftl;
use crate::nand::NandGeometry;
use crate::queue::BlockQueue;
use crate::Trace;
use std::num::NonZeroUsize;

struct RamDisk {
    queue: BlockQueue,
    trace: Trace,
    flushes: usize,
    cut_at: Option<usize>,
}

impl RamDisk {
    fn new(blocks: usize) -> Self {
        let geometry = NandGeometry::new(20, 8).unwrap();
        Self::with_ftl(Ftl::with_geometry(blocks, geometry).unwrap())
    }

    fn with_ftl(ftl: Ftl) -> Self {
        Self {
            queue: BlockQueue::new(ftl, NonZeroUsize::MIN),
            trace: Vec::new(),
            flushes: 0,
            cut_at: None,
        }
    }

    fn cut(self) -> Self {
        Self::with_ftl(Ftl::open(self.queue.power_cut(), &mut Vec::new()).unwrap())
    }
}

impl Disk for RamDisk {
    fn block_count(&self) -> u16 {
        self.queue.logical_blocks() as u16
    }

    fn read_block(&mut self, lba: u16) -> io::Result<Block> {
        self.queue.read(usize::from(lba), &mut self.trace)
    }

    fn write_block(&mut self, lba: u16, block: Block) -> io::Result<()> {
        self.queue.write(usize::from(lba), block, &mut self.trace)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.queue.flush(&mut self.trace)?;
        self.flushes += 1;
        if self.cut_at == Some(self.flushes) {
            return Err(io::Error::other("power cut after flush"));
        }
        Ok(())
    }
}

fn replacement() -> (JournalFs<RamDisk>, u64, u64) {
    let mut fs = JournalFs::format(RamDisk::new(64)).unwrap();
    let old = fs.create(b"events.log", 0o640).unwrap();
    fs.write(old, 0, b"old").unwrap();
    fs.sync_dir().unwrap();
    let new = fs.create(b"replacement", 0o600).unwrap();
    let contents = vec![42; BLOCK_BYTES + 1];
    fs.write(new, 0, &contents).unwrap();
    fs.sync_file(new).unwrap();
    fs.rename(b"replacement", b"events.log", true).unwrap();
    (fs, old, new)
}

#[test]
fn cuts_at_every_barrier_keep_exactly_one_root_and_its_allocation() {
    // 0 is before the transaction, followed by data, redo, commit, home and clear.
    for boundary in 0..=5 {
        let (mut fs, old, new) = replacement();
        fs.disk.flushes = 0;
        fs.disk.cut_at = Some(boundary);
        if boundary != 0 {
            assert_eq!(
                fs.sync_dir().unwrap_err().to_string(),
                "power cut after flush"
            );
            assert!(fs.create(b"later", 0o600).is_err());
        }
        let mut recovered = JournalFs::open(fs.into_disk().cut()).unwrap();
        assert_eq!(recovered.entries().unwrap().count(), 1);
        let file = recovered.lookup(b"events.log").unwrap();
        if boundary < 3 {
            assert_eq!(file.ino, old, "boundary {boundary}");
            assert_eq!(recovered.read(old, 0, 99).unwrap(), b"old");
            assert_eq!(recovered.free_blocks(), 60);
        } else {
            assert_eq!(file.ino, new, "boundary {boundary}");
            assert_eq!(
                recovered.read(new, 0, BLOCK_BYTES + 2).unwrap(),
                vec![42; BLOCK_BYTES + 1]
            );
            assert_eq!(recovered.free_blocks(), 59);
        }
        let generation = recovered.generation();
        let again = JournalFs::open(recovered.into_disk().cut()).unwrap();
        assert_eq!(again.generation(), generation);
        assert_eq!(again.lookup(b"events.log").unwrap(), file);
    }
}

#[test]
fn a_second_cut_during_replay_preserves_the_committed_update() {
    for boundary in [1, 2] {
        let (mut fs, _, new) = replacement();
        fs.disk.flushes = 0;
        fs.disk.cut_at = Some(3);
        assert_eq!(
            fs.sync_dir().unwrap_err().to_string(),
            "power cut after flush"
        );
        let mut disk = fs.into_disk().cut();
        disk.cut_at = Some(boundary);
        assert_eq!(
            recover_metadata(&mut disk).err().unwrap().to_string(),
            "power cut after flush"
        );
        let mut recovered = JournalFs::open(disk.cut()).unwrap();
        assert_eq!(recovered.entries().unwrap().count(), 1);
        assert_eq!(recovered.lookup(b"events.log").unwrap().ino, new);
        assert_eq!(
            recovered.read(new, 0, BLOCK_BYTES + 2).unwrap(),
            vec![42; BLOCK_BYTES + 1]
        );
        assert_eq!(recovered.free_blocks(), 59);
        let generation = recovered.generation();
        let again = JournalFs::open(recovered.into_disk().cut()).unwrap();
        assert_eq!(again.generation(), generation);
        assert_eq!(again.disk.flushes, 0, "clean reopen should need no replay");
    }
}

#[test]
fn full_device_preserves_a_partial_block_and_reclaims_an_aligned_shrink() {
    let mut fs = JournalFs::format(RamDisk::new(8)).unwrap();
    let ino = fs.create(b"full", 0o600).unwrap();
    let data = vec![0x72; 5 * BLOCK_BYTES];
    fs.write(ino, 0, &data).unwrap();
    fs.sync_dir().unwrap();
    assert_eq!(fs.free_blocks(), 0);

    let prefix = 4 * BLOCK_BYTES;
    let error = fs.truncate(ino, (prefix + 3) as u64).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::StorageFull);
    assert_eq!(fs.stat(ino).unwrap().len, data.len());
    assert_eq!(fs.read(ino, 0, data.len()).unwrap(), data);
    assert_eq!(fs.free_blocks(), 0);
    let mut fs = JournalFs::open(fs.into_disk().cut()).unwrap();
    assert_eq!(fs.read(ino, 0, data.len()).unwrap(), data);

    fs.truncate(ino, prefix as u64).unwrap();
    assert_eq!(fs.free_blocks(), 0, "the old durable extent is still live");
    fs.sync_file(ino).unwrap();
    let mut recovered = JournalFs::open(fs.into_disk().cut()).unwrap();
    assert_eq!(recovered.stat(ino).unwrap().blocks, 4);
    assert_eq!(recovered.free_blocks(), 1);
    assert_eq!(recovered.read(ino, 0, data.len()).unwrap(), data[..prefix]);
}

#[test]
fn corruption_of_committed_redo_or_marker_fails_reopen() {
    for lba in [REDO, COMMIT] {
        let (mut fs, _, _) = replacement();
        fs.disk.flushes = 0;
        fs.disk.cut_at = Some(3);
        assert!(fs.sync_dir().is_err());
        let mut disk = fs.into_disk().cut();
        let mut damaged = disk.read_block(lba).unwrap();
        damaged[72] ^= 1;
        disk.write_block(lba, damaged).unwrap();
        disk.flush().unwrap();
        let error = JournalFs::open(disk.cut())
            .err()
            .expect("corruption detected");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}

#[test]
fn the_marker_rejects_a_different_valid_redo_at_the_same_generation() {
    let (mut fs, _, _) = replacement();
    fs.disk.flushes = 0;
    fs.disk.cut_at = Some(3);
    assert!(fs.sync_dir().is_err());
    let mut disk = fs.into_disk().cut();
    let block = disk.read_block(REDO).unwrap();
    let mut redo = Metadata::decode(&block, disk.block_count()).unwrap();
    redo.files.get_mut(b"events.log".as_slice()).unwrap().mode ^= 0o100;
    disk.write_block(REDO, redo.encode()).unwrap();
    disk.flush().unwrap();
    assert_replay_rejected(disk.cut());
}

#[test]
fn the_marker_rejects_a_valid_redo_from_another_generation() {
    let (mut fs, _, _) = replacement();
    fs.disk.flushes = 0;
    fs.disk.cut_at = Some(3);
    assert!(fs.sync_dir().is_err());
    let mut disk = fs.into_disk().cut();
    let redo = disk.read_block(REDO).unwrap();
    let metadata = Metadata::decode(&redo, disk.block_count()).unwrap();
    let wrong_generation = metadata.generation.checked_add(1).unwrap();
    disk.write_block(
        COMMIT,
        commit_record(wrong_generation, frame_checksum(&redo)),
    )
    .unwrap();
    disk.flush().unwrap();
    assert_replay_rejected(disk.cut());
}

fn assert_replay_rejected(mut disk: RamDisk) {
    let home = disk.read_block(HOME).unwrap();
    let marker = disk.read_block(COMMIT).unwrap();
    let error = recover_metadata(&mut disk)
        .err()
        .expect("marker must bind the original redo and generation");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(disk.flushes, 0, "an invalid redo must not reach replay");
    assert!(disk.read_block(HOME).unwrap() == home, "home block changed");
    assert!(disk.read_block(COMMIT).unwrap() == marker, "marker changed");
}

#[test]
fn file_sync_keeps_the_durable_name_and_directory_sync_publishes_rename() {
    let mut fs = JournalFs::format(RamDisk::new(64)).unwrap();
    let log = fs.create(b"log", 0o600).unwrap();
    fs.write(log, 0, b"first").unwrap();
    fs.sync_dir().unwrap();
    fs.rename(b"log", b"moved", true).unwrap();
    fs.write(log, 5, b"second").unwrap();
    fs.sync_file(log).unwrap();
    let scratch = fs.create(b"scratch", 0o600).unwrap();
    fs.write(scratch, 0, b"unlinked after cut").unwrap();
    fs.sync_file(scratch).unwrap();

    let mut fs = JournalFs::open(fs.into_disk().cut()).unwrap();
    assert_eq!(fs.read(log, 0, 20).unwrap(), b"firstsecond");
    assert_eq!(fs.lookup(b"log").unwrap().ino, log);
    assert!(fs.lookup(b"moved").is_err());
    assert!(fs.lookup(b"scratch").is_err());
    fs.rename(b"log", b"moved", true).unwrap();
    fs.sync_dir().unwrap();
    let fs = JournalFs::open(fs.into_disk().cut()).unwrap();
    assert_eq!(fs.lookup(b"moved").unwrap().ino, log);
}

#[test]
fn copy_on_write_preserves_durable_bytes_and_truncate_zero_fills() {
    let mut fs = JournalFs::format(RamDisk::new(64)).unwrap();
    let ino = fs.create(b"one", 0o600).unwrap();
    fs.write(ino, 0, b"123456").unwrap();
    fs.sync_dir().unwrap();
    fs.write(ino, 1, b"overwrite").unwrap();
    fs.write(ino, 0, b"another overwrite").unwrap();
    let other = fs.create(b"two", 0o600).unwrap();
    fs.sync_file(other).unwrap();
    let mut fs = JournalFs::open(fs.into_disk().cut()).unwrap();
    assert_eq!(fs.read(ino, 0, 20).unwrap(), b"123456");
    fs.truncate(ino, 3).unwrap();
    fs.truncate(ino, BLOCK_BYTES as u64 + 2).unwrap();
    assert_eq!(fs.read(ino, 0, 7).unwrap(), b"123\0\0\0\0");
    assert_eq!(fs.read(ino, BLOCK_BYTES as u64 - 1, 4).unwrap(), [0; 3]);
    fs.sync_file(ino).unwrap();
    let mut fs = JournalFs::open(fs.into_disk().cut()).unwrap();
    assert_eq!(fs.stat(ino).unwrap().blocks, 1);
    assert_eq!(fs.read(ino, BLOCK_BYTES as u64 - 1, 4).unwrap(), [0; 3]);
}

#[test]
fn replacement_keeps_an_open_victim_and_releases_its_allocation() {
    let mut fs = JournalFs::format(RamDisk::new(5)).unwrap();
    let old = fs.create(b"old", 0o600).unwrap();
    let new = fs.create(b"new", 0o600).unwrap();
    fs.write(old, 0, b"first").unwrap();
    fs.write(new, 0, b"second").unwrap();
    fs.sync_dir().unwrap();
    assert_eq!(
        fs.write(old, 1, b"cannot fit").unwrap_err().kind(),
        io::ErrorKind::StorageFull
    );
    fs.retain_file(old).unwrap();
    fs.rename(b"new", b"old", true).unwrap();
    fs.sync_dir().unwrap();
    assert!(!fs.stat(old).unwrap().linked);
    assert_eq!(fs.read(old, 0, 20).unwrap(), b"first");
    assert_eq!(fs.free_blocks(), 0);
    fs.release_file(old);
    assert!(fs.stat(old).is_err());
    assert_eq!(fs.free_blocks(), 1);
    fs.write(new, 0, b"second!").unwrap();
    fs.sync_file(new).unwrap();
    let mut fs = JournalFs::open(fs.into_disk().cut()).unwrap();
    assert_eq!(fs.read(new, 0, 20).unwrap(), b"second!");
    assert_eq!(fs.free_blocks(), 1);
}
