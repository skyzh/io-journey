use std::collections::VecDeque;
use std::io;
use std::num::NonZeroUsize;

use crate::ftl::{check_lba, FlashImage, Ftl};
use crate::nand::Block;
use crate::Trace;

enum Operation {
    Read(usize),
    Write(usize, Block),
    Flush,
}

impl Operation {
    fn label(&self) -> String {
        match self {
            Self::Read(lba) => format!("read LBA {lba}"),
            Self::Write(lba, _) => format!("write LBA {lba}"),
            Self::Flush => "flush".into(),
        }
    }
}

struct Request {
    id: u64,
    operation: Operation,
}

struct Completion {
    id: u64,
    data: Option<Block>,
}

pub(crate) struct BlockQueue {
    ftl: Ftl,
    pending: VecDeque<Request>,
    depth: NonZeroUsize,
    next_id: u64,
    failed: bool,
}

impl BlockQueue {
    pub(crate) fn new(ftl: Ftl, depth: NonZeroUsize) -> Self {
        Self {
            ftl,
            pending: VecDeque::new(),
            depth,
            next_id: 1,
            failed: false,
        }
    }

    pub(crate) fn read(&mut self, lba: usize, trace: &mut Trace) -> io::Result<Block> {
        check_lba(lba)?;
        let id = self.submit(Operation::Read(lba), trace)?;
        Ok(self.wait(id, trace)?.data.expect("read completion"))
    }

    /// Accept a write. The device executes it when the queue advances.
    pub(crate) fn write(&mut self, lba: usize, data: Block, trace: &mut Trace) -> io::Result<()> {
        check_lba(lba)?;
        self.submit(Operation::Write(lba, data), trace)?;
        Ok(())
    }

    pub(crate) fn flush(&mut self, trace: &mut Trace) -> io::Result<()> {
        let id = self.submit(Operation::Flush, trace)?;
        self.wait(id, trace)?;
        Ok(())
    }

    pub(crate) fn power_cut(self) -> FlashImage {
        self.ftl.power_cut()
    }

    pub(crate) fn tear_next_program(&mut self, bytes: usize) -> io::Result<()> {
        self.ftl.tear_next_program(bytes)
    }

    fn submit(&mut self, operation: Operation, trace: &mut Trace) -> io::Result<u64> {
        if self.failed {
            return Err(io::Error::other(
                "queue stopped after a device failure. Reopen the image",
            ));
        }
        let id = self.next_id;
        let successor = id
            .checked_add(1)
            .ok_or_else(|| io::Error::other("request IDs exhausted"))?;
        if self.pending.len() == self.depth.get() {
            trace.push(format!(
                "queue  backpressure at depth {}. Completing the oldest request",
                self.depth
            ));
            self.complete_next(trace)?;
        }
        trace.push(format!(
            "queue  submit #{id} {} ({}/{})",
            operation.label(),
            self.pending.len() + 1,
            self.depth
        ));
        self.next_id = successor;
        self.pending.push_back(Request { id, operation });
        Ok(id)
    }

    fn wait(&mut self, id: u64, trace: &mut Trace) -> io::Result<Completion> {
        while let Some(completion) = self.complete_next(trace)? {
            if completion.id == id {
                return Ok(completion);
            }
        }
        Err(io::Error::other("queued request has no completion"))
    }

    fn complete_next(&mut self, trace: &mut Trace) -> io::Result<Option<Completion>> {
        let Some(Request { id, operation }) = self.pending.pop_front() else {
            return Ok(None);
        };
        let label = operation.label();
        trace.push(format!("queue  dispatch #{id} {label}"));
        let outcome = match operation {
            Operation::Read(lba) => self.ftl.read(lba, trace).map(Some),
            Operation::Write(lba, data) => self.ftl.write(lba, data, trace).map(|()| None),
            Operation::Flush => {
                self.ftl.flush(trace);
                Ok(None)
            }
        };
        match outcome {
            Ok(data) => {
                trace.push(format!("queue  complete #{id} {label}"));
                Ok(Some(Completion { id, data }))
            }
            Err(error) => {
                let dropped = self.pending.len();
                self.pending.clear();
                self.failed = true;
                trace.push(format!(
                    "queue  failed #{id} {label} (discarded {dropped} queued requests)"
                ));
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ftl::LOGICAL_BLOCKS;
    use crate::nand::BLOCK_BYTES;

    #[test]
    fn full_queue_makes_room_and_flush_excludes_a_later_write() {
        let mut trace = Vec::new();
        let mut queue = BlockQueue::new(Ftl::new(), NonZeroUsize::new(2).unwrap());
        queue
            .write(0, Box::new([1; BLOCK_BYTES]), &mut trace)
            .unwrap();
        let flush = queue.submit(Operation::Flush, &mut trace).unwrap();
        queue
            .write(0, Box::new([2; BLOCK_BYTES]), &mut trace)
            .unwrap();
        assert!(trace
            .iter()
            .any(|event| event.contains("backpressure at depth 2")));
        assert_eq!(queue.pending.len(), 2);
        queue.wait(flush, &mut trace).unwrap();
        assert_eq!(queue.pending.len(), 1);
        assert_eq!(*queue.read(0, &mut trace).unwrap(), [2; BLOCK_BYTES]);

        let ftl = Ftl::open(queue.power_cut(), &mut trace).unwrap();
        let mut reopened = BlockQueue::new(ftl, NonZeroUsize::MIN);
        assert_eq!(*reopened.read(0, &mut trace).unwrap(), [1; BLOCK_BYTES]);
    }

    #[test]
    fn invalid_write_leaves_a_full_queue_untouched() {
        let mut trace = Vec::new();
        let mut queue = BlockQueue::new(Ftl::new(), NonZeroUsize::MIN);
        queue
            .write(0, Box::new([1; BLOCK_BYTES]), &mut trace)
            .unwrap();
        let events = trace.len();

        let err = queue
            .write(LOGICAL_BLOCKS, Box::new([2; BLOCK_BYTES]), &mut trace)
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(trace.len(), events);
        assert_eq!(queue.pending.len(), 1);
        assert_eq!(*queue.read(0, &mut trace).unwrap(), [1; BLOCK_BYTES]);
    }
}
