use super::{BlockQueue, Request, RequestId, SubmitError};
use crate::Trace;

impl BlockQueue {
    pub(super) fn has_room(&self) -> bool {
        // TODO: Leave the accepted queue within its configured depth.
        true
    }

    pub(super) fn allocate_id(&mut self) -> Result<RequestId, SubmitError> {
        // TODO: Reserve the next ID without wrapping or consuming it on error.
        Ok(self.next_id)
    }

    pub(super) fn take_next_request(&mut self) -> Option<Request> {
        // TODO: Complete requests in submission order.
        self.pending.pop_back()
    }

    pub(super) fn flush_image(&mut self, _trace: &mut Trace) {
        // TODO: Publish the durable image at this request's place in the queue.
    }
}
