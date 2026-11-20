use std::io;

use super::format::decode_commit;
use super::{Block, Disk, Metadata, COMMIT, HOME};

pub(super) fn persist_redo(_disk: &mut impl Disk, _redo: &Block, _marker: Block) -> io::Result<()> {
    // TODO: Make data, redo, and the commit marker durable in that order.
    Err(io::Error::other("implement journal barriers"))
}

pub(super) fn read_committed(disk: &mut impl Disk) -> io::Result<(Metadata, Option<Block>)> {
    let marker = disk.read_block(COMMIT)?;
    let Some((_generation, _checksum)) = decode_commit(&marker)? else {
        let home = disk.read_block(HOME)?;
        return Ok((Metadata::decode(&home, disk.block_count())?, None));
    };
    // TODO: Check the committed redo and return it for replay.
    Err(io::Error::other("implement committed redo replay"))
}
