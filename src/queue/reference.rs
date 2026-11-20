use std::io;

use super::{BlockQueue, Request, RequestId, SubmitError};
use crate::Trace;

impl BlockQueue {
    pub(super) fn has_room(&self) -> bool {
        self.pending.len() < self.depth.get()
    }

    pub(super) fn allocate_id(&mut self) -> Result<RequestId, SubmitError> {
        let id = self.next_id;
        self.next_id = id
            .checked_add(1)
            .ok_or_else(|| SubmitError::Rejected(io::Error::other("request IDs exhausted")))?;
        Ok(id)
    }

    pub(super) fn take_next_request(&mut self) -> Option<Request> {
        self.pending.pop_front()
    }

    pub(super) fn flush_image(&mut self, trace: &mut Trace) {
        self.ftl.flush(trace);
    }
}
