use super::*;
use crate::record::crc32;

const METADATA_MAGIC: &[u8; 8] = b"IOJFS02\0";
const COMMIT_MAGIC: &[u8; 8] = b"IOJTX02\0";
const HEADER_BYTES: usize = 32;
const NAME_OFFSET: usize = 16;
const BLOCKS_OFFSET: usize = NAME_OFFSET + MAX_NAME_BYTES;
const INODE_BYTES: usize = BLOCKS_OFFSET + FILE_BLOCKS * 2;
const CHECKSUM_OFFSET: usize = BLOCK_BYTES - 4;
const _: () = assert!(HEADER_BYTES + MAX_FILES * INODE_BYTES <= CHECKSUM_OFFSET);

#[derive(Clone)]
pub(super) struct Metadata {
    pub generation: u64,
    pub next_inode: u64,
    pub files: BTreeMap<Vec<u8>, Node>,
}

impl Metadata {
    pub fn empty() -> Self {
        Self {
            generation: 0,
            next_inode: ROOT_INODE + 1,
            files: BTreeMap::new(),
        }
    }

    pub fn encode(&self) -> Block {
        let mut block = Box::new([0; BLOCK_BYTES]);
        block[..8].copy_from_slice(METADATA_MAGIC);
        block[8..16].copy_from_slice(&self.generation.to_le_bytes());
        block[16..24].copy_from_slice(&self.next_inode.to_le_bytes());
        block[24..26].copy_from_slice(&(self.files.len() as u16).to_le_bytes());
        let entries = block[HEADER_BYTES..CHECKSUM_OFFSET].chunks_exact_mut(INODE_BYTES);
        for ((name, node), entry) in self.files.iter().zip(entries) {
            entry[..8].copy_from_slice(&node.ino.to_le_bytes());
            entry[8..12].copy_from_slice(&(node.len as u32).to_le_bytes());
            entry[12..14].copy_from_slice(&node.mode.to_le_bytes());
            entry[14..16].copy_from_slice(&(name.len() as u16).to_le_bytes());
            entry[NAME_OFFSET..NAME_OFFSET + name.len()].copy_from_slice(name);
            for (&lba, bytes) in node
                .blocks
                .iter()
                .zip(entry[BLOCKS_OFFSET..].chunks_exact_mut(2))
            {
                bytes.copy_from_slice(&lba.to_le_bytes());
            }
        }
        seal(&mut block);
        block
    }

    pub fn decode(block: &[u8; BLOCK_BYTES], capacity: u16) -> io::Result<Self> {
        check_frame(block, METADATA_MAGIC)?;
        let generation = u64::from_le_bytes(block[8..16].try_into().unwrap());
        let next_inode = u64::from_le_bytes(block[16..24].try_into().unwrap());
        let count = usize::from(u16::from_le_bytes(block[24..26].try_into().unwrap()));
        if next_inode <= ROOT_INODE || count > MAX_FILES {
            return Err(corrupt("invalid directory header"));
        }

        let mut files = BTreeMap::new();
        let mut inodes = BTreeSet::new();
        let mut allocated = BTreeSet::new();
        for entry in block[HEADER_BYTES..].chunks_exact(INODE_BYTES).take(count) {
            let ino = u64::from_le_bytes(entry[..8].try_into().unwrap());
            let len = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
            let mode = u16::from_le_bytes(entry[12..14].try_into().unwrap());
            let name_len = usize::from(u16::from_le_bytes(entry[14..16].try_into().unwrap()));
            if !(ROOT_INODE + 1..next_inode).contains(&ino)
                || !inodes.insert(ino)
                || len > MAX_FILE_BYTES
                || name_len > MAX_NAME_BYTES
                || mode & !0o777 != 0
            {
                return Err(corrupt("invalid or duplicate inode"));
            }
            let name = &entry[NAME_OFFSET..NAME_OFFSET + name_len];
            check_name(name).map_err(|_| corrupt("invalid directory name"))?;
            let mut blocks = [0; FILE_BLOCKS];
            let visible_blocks = len.div_ceil(BLOCK_BYTES);
            for (index, bytes) in entry[BLOCKS_OFFSET..].chunks_exact(2).enumerate() {
                let lba = u16::from_le_bytes(bytes.try_into().unwrap());
                if lba != 0
                    && (index >= visible_blocks
                        || !(FIRST_DATA..capacity).contains(&lba)
                        || !allocated.insert(lba))
                {
                    return Err(corrupt("invalid or duplicate block allocation"));
                }
                blocks[index] = lba;
            }
            if files
                .insert(
                    name.to_vec(),
                    Node {
                        ino,
                        len,
                        mode,
                        blocks,
                    },
                )
                .is_some()
            {
                return Err(corrupt("duplicate directory name"));
            }
        }
        Ok(Self {
            generation,
            next_inode,
            files,
        })
    }
}

pub(super) fn commit_record(generation: u64, redo_checksum: u32) -> Block {
    let mut block = Box::new([0; BLOCK_BYTES]);
    block[..8].copy_from_slice(COMMIT_MAGIC);
    block[8..16].copy_from_slice(&generation.to_le_bytes());
    block[16..20].copy_from_slice(&redo_checksum.to_le_bytes());
    seal(&mut block);
    block
}

pub(super) fn decode_commit(block: &[u8; BLOCK_BYTES]) -> io::Result<Option<(u64, u32)>> {
    if block.iter().all(|&byte| byte == 0) {
        return Ok(None);
    }
    check_frame(block, COMMIT_MAGIC)?;
    let generation = u64::from_le_bytes(block[8..16].try_into().unwrap());
    let checksum = u32::from_le_bytes(block[16..20].try_into().unwrap());
    if generation == 0 {
        return Err(corrupt("invalid journal commit generation"));
    }
    Ok(Some((generation, checksum)))
}

fn seal(block: &mut [u8; BLOCK_BYTES]) {
    let checksum = frame_checksum(block);
    block[CHECKSUM_OFFSET..].copy_from_slice(&checksum.to_le_bytes());
}

pub(super) fn frame_checksum(block: &[u8; BLOCK_BYTES]) -> u32 {
    // Exclude the stored CRC, whose inclusion produces a fixed CRC residue
    // across otherwise valid frames.
    crc32(&block[..CHECKSUM_OFFSET])
}

fn check_frame(block: &[u8; BLOCK_BYTES], magic: &[u8; 8]) -> io::Result<()> {
    let checksum = u32::from_le_bytes(block[CHECKSUM_OFFSET..].try_into().unwrap());
    if &block[..8] != magic || frame_checksum(block) != checksum {
        return Err(corrupt("metadata frame magic or checksum mismatch"));
    }
    Ok(())
}
