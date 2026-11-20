use std::io;

use super::format::decode_commit;
use super::{corrupt, frame_checksum, Block, Disk, Metadata, COMMIT, HOME, REDO};

pub(super) fn persist_redo(disk: &mut impl Disk, redo: &Block, marker: Block) -> io::Result<()> {
    disk.flush()?;
    disk.write_block(REDO, redo.clone())?;
    disk.flush()?;
    disk.write_block(COMMIT, marker)?;
    disk.flush()
}

pub(super) fn read_committed(disk: &mut impl Disk) -> io::Result<(Metadata, Option<Block>)> {
    let marker = disk.read_block(COMMIT)?;
    let Some((generation, checksum)) = decode_commit(&marker)? else {
        let home = disk.read_block(HOME)?;
        return Ok((Metadata::decode(&home, disk.block_count())?, None));
    };
    let redo = disk.read_block(REDO)?;
    if frame_checksum(&redo) != checksum {
        return Err(corrupt("committed redo checksum mismatch"));
    }
    let metadata = Metadata::decode(&redo, disk.block_count())?;
    if metadata.generation != generation {
        return Err(corrupt("committed redo generation mismatch"));
    }
    Ok((metadata, Some(redo)))
}
