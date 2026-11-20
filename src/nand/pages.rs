use super::Block;

#[cfg(feature = "shared-pages")]
use std::sync::Arc;

#[cfg(not(feature = "shared-pages"))]
pub(super) type StoredPage = Block;
#[cfg(feature = "shared-pages")]
pub(super) type StoredPage = Arc<Block>;

#[cfg(not(feature = "shared-pages"))]
pub(super) fn keep(data: Block) -> StoredPage {
    data
}

#[cfg(feature = "shared-pages")]
pub(super) fn keep(data: Block) -> StoredPage {
    Arc::new(data)
}

pub(super) fn borrow(page: &StoredPage) -> &Block {
    page
}
