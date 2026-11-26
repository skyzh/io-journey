use super::{Block, StoredPage};
use std::io;

pub(super) fn read_page(_pages: &[Option<StoredPage>], _page: usize) -> Option<&Block> {
    None
}

pub(super) fn program_page(
    _pages: &mut [Option<StoredPage>],
    _page: usize,
    _data: Block,
) -> io::Result<()> {
    Err(io::Error::other("implement NAND page programming"))
}

pub(super) fn erase_pages(_pages: &mut [Option<StoredPage>]) {}
