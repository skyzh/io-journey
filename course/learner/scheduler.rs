use super::{Policy, Staged};
use crate::dispatch::Kind;

pub(super) fn choose_request(
    _policy: Policy,
    _staged: &[Staged],
    _remaining_bypasses: usize,
) -> usize {
    0
}

pub(super) fn after_dispatch(remaining_bypasses: usize, _index: usize, _kind: Kind) -> usize {
    remaining_bypasses
}
