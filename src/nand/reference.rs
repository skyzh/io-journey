use super::{storage, Block, StoredPage};
use std::io;

pub(super) fn read_page(pages: &[Option<StoredPage>], page: usize) -> Option<&Block> {
    pages.get(page)?.as_ref().map(storage::borrow)
}

pub(super) fn program_page(
    pages: &mut [Option<StoredPage>],
    page: usize,
    data: Block,
) -> io::Result<()> {
    let slot = pages
        .get_mut(page)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "physical page out of range"))?;
    if slot.is_some() {
        return Err(io::Error::other(
            "NAND page must be erased before programming",
        ));
    }
    *slot = Some(storage::keep(data));
    Ok(())
}

pub(super) fn erase_pages(pages: &mut [Option<StoredPage>]) {
    pages.fill(None);
}
