use super::{BlockQueue, Request, RequestId, SubmitError};
use crate::Trace;

impl BlockQueue {
    pub(super) fn has_room(&self) -> bool {
        // pending is a VecDeque<Request>. depth is a NonZeroUsize.
        // TODO: Leave the accepted queue within its configured depth.
        true
    }

    pub(super) fn allocate_id(&mut self) -> Result<RequestId, SubmitError> {
        // RequestId is u64. SubmitError::Rejected holds a std::io::Error.
        // TODO: Reserve the next ID without wrapping or consuming it on error.
        Ok(self.next_id)
    }

    pub(super) fn take_next_request(&mut self) -> Option<Request> {
        // TODO: Complete requests in submission order.
        self.pending.pop_back()
    }

    pub(super) fn flush_image(&mut self, _trace: &mut Trace) {
        // The supplied Ftl::flush takes &mut Trace and returns ().
        // TODO: Publish the durable image at this request's place in the queue.
    }
}
