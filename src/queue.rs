use std::collections::VecDeque;
use std::io;
use std::num::NonZeroUsize;

use crate::ftl::{FlashImage, Ftl};
use crate::nand::Block;
use crate::Trace;

#[cfg_attr(feature = "learner-queue", path = "../course/learner/queue.rs")]
#[cfg_attr(not(feature = "learner-queue"), path = "queue/reference.rs")]
mod behavior;

pub type RequestId = u64;

#[derive(Debug)]
pub enum Operation {
    Read { lba: usize, blocks: usize },
    Write { lba: usize, data: Vec<Block> },
    Flush,
}

impl Operation {
    fn span(&self) -> Option<(usize, usize)> {
        match self {
            Self::Read { lba, blocks } => Some((*lba, *blocks)),
            Self::Write { lba, data } => Some((*lba, data.len())),
            Self::Flush => None,
        }
    }

    fn label(&self) -> String {
        match self {
            Self::Read { lba, blocks } => format_span("read", *lba, *blocks),
            Self::Write { lba, data } => format_span("write", *lba, data.len()),
            Self::Flush => "flush".into(),
        }
    }
}

fn format_span(verb: &str, lba: usize, blocks: usize) -> String {
    if blocks == 1 {
        format!("{verb} LBA {lba}")
    } else {
        format!("{verb} {blocks} blocks from LBA {lba}")
    }
}

#[derive(Debug)]
pub enum SubmitError {
    Full(Submission),
    Rejected(io::Error),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Driver {
    Explicit,
    Filesystem,
}

#[derive(Debug)]
pub struct Submission {
    operation: Operation,
    driver: Driver,
}

impl From<Operation> for Submission {
    fn from(operation: Operation) -> Self {
        Self {
            operation,
            driver: Driver::Explicit,
        }
    }
}

#[derive(Debug)]
pub enum Response {
    Read(Vec<Block>),
    Done,
}

struct Request {
    id: RequestId,
    submission: Submission,
}

pub struct Completion {
    pub id: RequestId,
    pub result: io::Result<Response>,
}

pub struct BlockQueue {
    ftl: Ftl,
    pending: VecDeque<Request>,
    depth: NonZeroUsize,
    next_id: RequestId,
    failed: bool,
}

impl BlockQueue {
    pub fn new(ftl: Ftl, depth: NonZeroUsize) -> Self {
        Self {
            ftl,
            pending: VecDeque::new(),
            depth,
            next_id: 1,
            failed: false,
        }
    }

    pub fn logical_blocks(&self) -> usize {
        self.ftl.logical_blocks()
    }

    pub fn read(&mut self, lba: usize, trace: &mut Trace) -> io::Result<Block> {
        Ok(self
            .read_range(lba, 1, trace)?
            .pop()
            .expect("one read block"))
    }

    pub fn read_range(
        &mut self,
        lba: usize,
        blocks: usize,
        trace: &mut Trace,
    ) -> io::Result<Vec<Block>> {
        let id = self.submit_driven(Operation::Read { lba, blocks }, trace)?;
        match self.wait(id, trace)? {
            Response::Read(data) => Ok(data),
            Response::Done => unreachable!("read completion"),
        }
    }

    /// Accept a write. The device executes it when the queue advances.
    pub fn write(&mut self, lba: usize, data: Block, trace: &mut Trace) -> io::Result<()> {
        self.write_range(lba, vec![data], trace)
    }

    pub fn write_range(
        &mut self,
        lba: usize,
        data: Vec<Block>,
        trace: &mut Trace,
    ) -> io::Result<()> {
        self.submit_driven(Operation::Write { lba, data }, trace)?;
        Ok(())
    }

    pub fn flush(&mut self, trace: &mut Trace) -> io::Result<()> {
        let id = self.submit_driven(Operation::Flush, trace)?;
        self.wait(id, trace)?;
        Ok(())
    }

    pub fn power_cut(self) -> FlashImage {
        self.ftl.power_cut()
    }

    /// Retain the last completed flush without flushing pending work.
    pub fn checkpoint(&self) -> FlashImage {
        self.ftl.checkpoint()
    }

    pub fn arm_torn_write(&mut self, lba: usize, prefix_bytes: usize) -> io::Result<()> {
        self.ftl.arm_torn_write(lba, prefix_bytes)
    }

    /// Accept one request without executing any queued work. On backpressure
    /// the caller gets the request back and can retry after a completion.
    pub fn try_submit(
        &mut self,
        submission: impl Into<Submission>,
        trace: &mut Trace,
    ) -> Result<RequestId, SubmitError> {
        let submission = submission.into();
        if let Some((lba, blocks)) = submission.operation.span() {
            let end = lba.checked_add(blocks);
            if blocks == 0 || end.is_none_or(|end| end > self.logical_blocks()) {
                return Err(SubmitError::Rejected(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("invalid range of {blocks} blocks from LBA {lba}"),
                )));
            }
        }
        if self.failed {
            return Err(SubmitError::Rejected(stopped_queue()));
        }
        self.require_driver(submission.driver)
            .map_err(SubmitError::Rejected)?;
        if !self.has_room() {
            return Err(SubmitError::Full(submission));
        }
        let id = self.allocate_id()?;
        trace.push(format!(
            "queue  submit #{id} {} ({}/{})",
            submission.operation.label(),
            self.pending.len() + 1,
            self.depth
        ));
        self.pending.push_back(Request { id, submission });
        Ok(id)
    }

    fn require_driver(&self, driver: Driver) -> io::Result<()> {
        if self
            .pending
            .front()
            .is_some_and(|request| request.submission.driver != driver)
        {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "complete the queued requests before switching drivers",
            ));
        }
        Ok(())
    }

    /// Advance the oldest request with its owning driver. The outer result
    /// rejects a driver mismatch. Execution errors retain the request ID.
    pub fn complete_next(
        &mut self,
        driver: Driver,
        trace: &mut Trace,
    ) -> io::Result<Option<Completion>> {
        self.require_driver(driver)?;
        let Some(request) = self.take_next_request() else {
            return Ok(None);
        };
        let id = request.id;
        let operation = request.submission.operation;
        let label = operation.label();
        if self.failed {
            trace.push(format!("queue  cancel #{id} {label}"));
            return Ok(Some(Completion {
                id,
                result: Err(stopped_queue()),
            }));
        }

        trace.push(format!("queue  dispatch #{id} {label}"));
        if let Some((_, blocks)) = operation.span().filter(|&(_, blocks)| blocks > 1) {
            trace.push(format!("queue  split #{id} into {blocks} 4 KiB blocks"));
        }
        let result = self.execute(operation, trace);
        if result.is_ok() {
            trace.push(format!("queue  complete #{id} {label}"));
        } else {
            self.failed = true;
            trace.push(format!("queue  failed #{id} {label}"));
        }
        Ok(Some(Completion { id, result }))
    }

    fn execute(&mut self, operation: Operation, trace: &mut Trace) -> io::Result<Response> {
        match operation {
            Operation::Read { lba, blocks } => (lba..lba + blocks)
                .map(|lba| self.ftl.read(lba, trace))
                .collect::<io::Result<Vec<_>>>()
                .map(Response::Read),
            Operation::Write { lba, data } => {
                for (offset, block) in data.into_iter().enumerate() {
                    self.ftl.write(lba + offset, block, trace)?;
                }
                Ok(Response::Done)
            }
            Operation::Flush => {
                self.flush_image(trace);
                Ok(Response::Done)
            }
        }
    }

    // Filesystem calls can drive their own work after explicit callers have
    // received all outstanding completions.
    fn submit_driven(&mut self, operation: Operation, trace: &mut Trace) -> io::Result<RequestId> {
        let mut submission = Submission {
            operation,
            driver: Driver::Filesystem,
        };
        loop {
            match self.try_submit(submission, trace) {
                Ok(id) => return Ok(id),
                Err(SubmitError::Rejected(error)) => return Err(error),
                Err(SubmitError::Full(request)) => {
                    trace.push(format!(
                        "queue  backpressure at depth {}. Completing the oldest request",
                        self.depth
                    ));
                    self.complete_next(Driver::Filesystem, trace)?
                        .expect("full queue")
                        .result?;
                    submission = request;
                }
            }
        }
    }

    fn wait(&mut self, id: RequestId, trace: &mut Trace) -> io::Result<Response> {
        while let Some(completion) = self.complete_next(Driver::Filesystem, trace)? {
            let value = completion.result?;
            if completion.id == id {
                return Ok(value);
            }
        }
        Err(io::Error::other("queued request has no completion"))
    }
}

fn stopped_queue() -> io::Error {
    io::Error::new(
        io::ErrorKind::BrokenPipe,
        "queue stopped after a device failure. Reopen the image",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ftl::{TornProgram, LOGICAL_BLOCKS};
    use crate::nand::BLOCK_BYTES;

    #[test]
    fn admission_returns_the_request_when_all_slots_are_occupied() {
        let mut queue = BlockQueue::new(Ftl::new(), NonZeroUsize::MIN);
        let mut trace = Vec::new();
        queue
            .try_submit(Operation::Read { lba: 0, blocks: 1 }, &mut trace)
            .unwrap();
        let result = queue.try_submit(Operation::Read { lba: 1, blocks: 1 }, &mut trace);
        assert!(matches!(
            result,
            Err(SubmitError::Full(Submission {
                operation: Operation::Read { lba: 1, blocks: 1 },
                driver: Driver::Explicit
            }))
        ));
        assert_eq!(queue.pending.len(), 1);
        assert_eq!(trace.len(), 1, "admission leaves queued work unexecuted");
    }

    #[test]
    fn accepted_requests_get_distinct_ordered_completions() {
        let mut queue = BlockQueue::new(Ftl::new(), NonZeroUsize::new(2).unwrap());
        let mut trace = Vec::new();
        let first = queue
            .try_submit(Operation::Read { lba: 0, blocks: 1 }, &mut trace)
            .unwrap();
        let second = queue
            .try_submit(Operation::Read { lba: 1, blocks: 1 }, &mut trace)
            .unwrap();
        assert_eq!(second, first + 1);
        for expected in [first, second] {
            let completion = queue
                .complete_next(Driver::Explicit, &mut trace)
                .unwrap()
                .unwrap();
            assert_eq!(completion.id, expected);
            assert!(completion.result.is_ok());
        }
        assert!(queue
            .complete_next(Driver::Explicit, &mut trace)
            .unwrap()
            .is_none());
    }

    fn write_request(lba: usize, values: &[u8]) -> Operation {
        Operation::Write {
            lba,
            data: values
                .iter()
                .map(|&byte| Box::new([byte; BLOCK_BYTES]))
                .collect(),
        }
    }

    #[test]
    fn a_bounded_queue_flushes_every_child_and_excludes_the_later_range() {
        let mut trace = Vec::new();
        let mut queue = BlockQueue::new(Ftl::new(), NonZeroUsize::new(2).unwrap());
        let earlier = queue
            .try_submit(write_request(0, &[1, 2, 3]), &mut trace)
            .unwrap();
        let flush = queue.try_submit(Operation::Flush, &mut trace).unwrap();

        let Err(SubmitError::Full(later)) = queue.try_submit(write_request(0, &[4, 5]), &mut trace)
        else {
            panic!("a third pending request must receive backpressure");
        };
        assert_eq!(queue.pending.len(), 2);
        let completion = queue
            .complete_next(Driver::Explicit, &mut trace)
            .unwrap()
            .unwrap();
        assert_eq!(completion.id, earlier);
        assert!(matches!(completion.result.unwrap(), Response::Done));
        let later = queue.try_submit(later, &mut trace).unwrap();
        assert_eq!(queue.pending.len(), 2);
        for id in [flush, later] {
            let completion = queue
                .complete_next(Driver::Explicit, &mut trace)
                .unwrap()
                .unwrap();
            assert_eq!(completion.id, id);
            assert!(matches!(completion.result.unwrap(), Response::Done));
        }
        let blocks = queue.read_range(0, 3, &mut trace).unwrap();
        for (block, byte) in blocks.iter().zip([4, 5, 3]) {
            assert_eq!(**block, [byte; BLOCK_BYTES]);
        }

        let ftl = Ftl::open(queue.power_cut(), &mut trace).unwrap();
        let mut reopened = BlockQueue::new(ftl, NonZeroUsize::MIN);
        let blocks = reopened.read_range(0, 3, &mut trace).unwrap();
        assert_eq!(blocks.len(), 3);
        for (block, byte) in blocks.iter().zip([1, 2, 3]) {
            assert_eq!(**block, [byte; BLOCK_BYTES]);
        }
        assert!(trace.iter().any(|line| line.contains("split #1 into 3")));
    }

    #[test]
    fn invalid_ranges_leave_a_full_queue_and_its_id_counter_untouched() {
        let mut trace = Vec::new();
        let mut queue = BlockQueue::new(Ftl::new(), NonZeroUsize::MIN);
        queue
            .write(0, Box::new([1; BLOCK_BYTES]), &mut trace)
            .unwrap();
        let events = trace.len();
        let next_id = queue.next_id;

        for operation in [
            Operation::Read {
                lba: usize::MAX,
                blocks: 2,
            },
            Operation::Read { lba: 0, blocks: 0 },
            write_request(LOGICAL_BLOCKS - 1, &[2, 3]),
            write_request(0, &[]),
        ] {
            let Err(SubmitError::Rejected(error)) = queue.try_submit(operation, &mut trace) else {
                panic!("an invalid range must be rejected before the queue advances");
            };
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        }
        assert_eq!(trace.len(), events);
        assert_eq!(queue.next_id, next_id);
        assert_eq!(queue.pending.len(), 1);
        assert_eq!(*queue.read(0, &mut trace).unwrap(), [1; BLOCK_BYTES]);
        assert_eq!(
            *queue.read(LOGICAL_BLOCKS - 1, &mut trace).unwrap(),
            [0; BLOCK_BYTES]
        );
    }

    fn check_explicit_read_survives_a_sync_flush(depth: NonZeroUsize) {
        let mut trace = Vec::new();
        let mut queue = BlockQueue::new(Ftl::new(), depth);
        queue
            .write(0, Box::new([0x42; BLOCK_BYTES]), &mut trace)
            .unwrap();
        queue.flush(&mut trace).unwrap();

        let read_id = queue
            .try_submit(Operation::Read { lba: 0, blocks: 1 }, &mut trace)
            .unwrap();
        let events = trace.len();
        let next_id = queue.next_id;
        let result = queue.flush(&mut trace);
        assert_eq!(
            result
                .expect_err("filesystem flush must leave the explicit read pending")
                .kind(),
            io::ErrorKind::WouldBlock,
        );
        assert_eq!(trace.len(), events);
        assert_eq!(queue.next_id, next_id);

        let completion = queue
            .complete_next(Driver::Explicit, &mut trace)
            .unwrap()
            .unwrap();
        assert_eq!(completion.id, read_id);
        let Response::Read(blocks) = completion.result.unwrap() else {
            panic!("expected the read payload");
        };
        assert_eq!(blocks.len(), 1);
        assert_eq!(*blocks[0], [0x42; BLOCK_BYTES]);
        assert!(queue
            .complete_next(Driver::Explicit, &mut trace)
            .unwrap()
            .is_none());
        queue.flush(&mut trace).unwrap();
    }

    #[test]
    fn sync_flush_yields_before_waiting_behind_an_explicit_read() {
        check_explicit_read_survives_a_sync_flush(NonZeroUsize::new(2).unwrap());
    }

    #[test]
    fn sync_flush_yields_before_advancing_a_full_explicit_queue() {
        check_explicit_read_survives_a_sync_flush(NonZeroUsize::MIN);
    }

    #[test]
    fn explicit_completion_yields_before_advancing_a_filesystem_write() {
        let mut trace = Vec::new();
        let depth = NonZeroUsize::MIN;
        let mut queue = BlockQueue::new(Ftl::new(), depth);
        queue
            .write(2, Box::new([0x73; BLOCK_BYTES]), &mut trace)
            .unwrap();
        let events = trace.len();

        let error = match queue.complete_next(Driver::Explicit, &mut trace) {
            Err(error) => error,
            Ok(_) => panic!("explicit completion must leave the filesystem write pending"),
        };
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(trace.len(), events);
        assert_eq!(queue.pending.len(), 1);
        queue.flush(&mut trace).unwrap();

        let ftl = Ftl::open(queue.power_cut(), &mut trace).unwrap();
        let mut reopened = BlockQueue::new(ftl, depth);
        assert_eq!(*reopened.read(2, &mut trace).unwrap(), [0x73; BLOCK_BYTES]);
    }

    #[test]
    fn a_failed_child_reports_one_error_and_cancels_the_flush_and_later_write() {
        let mut trace = Vec::new();
        let mut queue = BlockQueue::new(Ftl::new(), NonZeroUsize::new(3).unwrap());
        queue
            .write_range(0, vec![Box::new([1; BLOCK_BYTES]); 2], &mut trace)
            .unwrap();
        queue.flush(&mut trace).unwrap();
        queue.arm_torn_write(1, 37).unwrap();

        let write = queue
            .try_submit(write_request(0, &[2, 3]), &mut trace)
            .unwrap();
        let flush = queue.try_submit(Operation::Flush, &mut trace).unwrap();
        let later = queue
            .try_submit(write_request(2, &[4]), &mut trace)
            .unwrap();

        let completion = queue
            .complete_next(Driver::Explicit, &mut trace)
            .unwrap()
            .unwrap();
        assert_eq!(completion.id, write);
        let error = completion.result.unwrap_err();
        assert_eq!(
            error
                .get_ref()
                .unwrap()
                .downcast_ref::<TornProgram>()
                .unwrap()
                .lba,
            1
        );
        assert_eq!(*queue.ftl.read(0, &mut trace).unwrap(), [2; BLOCK_BYTES]);
        for id in [flush, later] {
            let completion = queue
                .complete_next(Driver::Explicit, &mut trace)
                .unwrap()
                .unwrap();
            assert_eq!(completion.id, id);
            assert_eq!(
                completion.result.unwrap_err().kind(),
                io::ErrorKind::BrokenPipe
            );
        }
        assert!(queue
            .complete_next(Driver::Explicit, &mut trace)
            .unwrap()
            .is_none());
        assert_eq!(
            queue.flush(&mut trace).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );

        let ftl = Ftl::open(queue.power_cut(), &mut trace).unwrap();
        let mut reopened = BlockQueue::new(ftl, NonZeroUsize::MIN);
        let blocks = reopened.read_range(0, 3, &mut trace).unwrap();
        assert_eq!(blocks.len(), 3);
        for (block, byte) in blocks.iter().zip([1, 1, 0]) {
            assert_eq!(**block, [byte; BLOCK_BYTES]);
        }
    }
}
