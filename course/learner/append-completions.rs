use std::io;

use super::{Completion, Request};

pub(super) fn check_completions(_wave: &[Request], _completed: Vec<Completion>) -> io::Result<()> {
    // TODO: Match every completion ID once and require the full byte count.
    Err(io::Error::other("implement completion validation"))
}

pub(super) fn drain_ring_wave(
    _wave: &[Request],
    _poll: impl FnMut(&mut Vec<Completion>) -> io::Result<()>,
) -> io::Result<()> {
    // TODO: Poll until every completion can be checked, then check the whole set.
    Err(io::Error::other("implement ring retirement"))
}
