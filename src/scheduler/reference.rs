use super::{Policy, Staged, READ_BYPASS_BUDGET};
use crate::dispatch::{self, Kind};

pub(super) fn choose_request(
    policy: Policy,
    staged: &[Staged],
    remaining_bypasses: usize,
) -> usize {
    if policy == Policy::PassThrough || remaining_bypasses == 0 {
        return 0;
    }
    dispatch::next_index(dispatch::Policy::ReadFirst, staged, |request| {
        request.request
    })
}

pub(super) fn after_dispatch(remaining_bypasses: usize, index: usize, kind: Kind) -> usize {
    match kind {
        Kind::Read if index != 0 => remaining_bypasses - 1,
        Kind::Read => remaining_bypasses,
        Kind::Write | Kind::Barrier => READ_BYPASS_BUDGET,
    }
}
