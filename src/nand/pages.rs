use super::Block;

pub(super) type StoredPage = Block;

pub(super) fn keep(data: Block) -> StoredPage {
    data
}

pub(super) fn borrow(page: &StoredPage) -> &Block {
    page
}
