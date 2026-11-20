use super::{DeviceRequest, Policy};

pub fn next_index<T>(
    _policy: Policy,
    _batch: &[T],
    _request: impl Fn(&T) -> DeviceRequest,
) -> usize {
    // TODO: Keep FIFO at the front. In read-first, prefer an eligible read before a barrier.
    0
}
