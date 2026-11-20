use super::Block;

// TODO: Store an immutable shared owner for each populated page.
pub(super) type StoredPage = Block;

pub(super) fn keep(data: Block) -> StoredPage {
    // TODO: Move the existing payload into the shared owner.
    data
}

pub(super) fn borrow(page: &StoredPage) -> &Block {
    page
}
