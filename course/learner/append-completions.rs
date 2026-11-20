use std::io;
use std::sync::Arc;

use super::{Completion, Request};

pub(super) fn check_completions(_wave: &[Request], _completed: Vec<Completion>) -> io::Result<()> {
    // TODO: Match every completion ID once and require the full byte count.
    Err(io::Error::other("implement completion validation"))
}

pub(super) fn retire_ring_wave(
    _wave: &[Request],
    _retained: &mut Vec<Arc<[u8]>>,
    _poll: impl FnMut(&mut Vec<Completion>) -> io::Result<()>,
) -> io::Result<()> {
    // TODO: Drain the wave before releasing the buffers proven retired.
    Err(io::Error::other("implement ring retirement"))
}
