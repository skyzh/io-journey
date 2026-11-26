use std::collections::VecDeque;
use std::io;
use std::num::NonZeroUsize;

use crate::dispatch::{DeviceRequest, Kind};
use crate::ftl::{FlashImage, Ftl};
use crate::nand::BLOCK_BYTES;
use crate::queue::{BlockQueue, Driver, Operation, RequestId, Response, SubmitError};
use crate::Trace;

#[cfg_attr(feature = "learner-scheduler", path = "../course/learner/scheduler.rs")]
#[cfg_attr(not(feature = "learner-scheduler"), path = "scheduler/reference.rs")]
mod policy;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    PassThrough,
    BoundedReadFirst,
}

const READ_BYPASS_BUDGET: usize = 1;

struct Staged {
    id: usize,
    request: DeviceRequest,
    operation: Operation,
}

pub struct Finished {
    pub id: usize,
    pub response: Response,
    pub wait_ticks: usize,
    pub completed_at: usize,
}

pub struct HostScheduler {
    device: BlockQueue,
    depth: NonZeroUsize,
    selection: Policy,
    failed: bool,
}

impl HostScheduler {
    pub fn new(ftl: Ftl, depth: NonZeroUsize, selection: Policy) -> Self {
        Self {
            device: BlockQueue::new(ftl, depth),
            depth,
            selection,
            failed: false,
        }
    }

    pub fn logical_blocks(&self) -> usize {
        self.device.logical_blocks()
    }

    pub fn power_cut(self) -> FlashImage {
        self.device.power_cut()
    }

    /// Treat this batch as ready at tick zero and collect completions.
    /// IDs are one-based positions in the batch. On error, reopen the
    /// last flushed image before submitting another batch.
    pub fn execute(
        &mut self,
        operations: impl IntoIterator<Item = Operation>,
    ) -> io::Result<Vec<Finished>> {
        if self.failed {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "host scheduler stopped after a failure; reopen the image",
            ));
        }
        let result = self.execute_ready(operations);
        self.failed = result.is_err();
        result
    }

    fn execute_ready(
        &mut self,
        operations: impl IntoIterator<Item = Operation>,
    ) -> io::Result<Vec<Finished>> {
        let mut staged = operations
            .into_iter()
            .enumerate()
            .map(|(index, operation)| {
                Ok(Staged {
                    id: index + 1,
                    request: describe(&operation)?,
                    operation,
                })
            })
            .collect::<io::Result<VecDeque<_>>>()?;
        let mut dispatched: VecDeque<(RequestId, usize, Kind)> = VecDeque::new();
        let mut finished = Vec::with_capacity(staged.len());
        let mut remaining_bypasses = READ_BYPASS_BUDGET;
        let mut elapsed = 0;
        let mut trace = Trace::new();

        while !staged.is_empty() || !dispatched.is_empty() {
            while !staged.is_empty() && dispatched.len() < self.depth.get() {
                let index = policy::choose_request(
                    self.selection,
                    staged.make_contiguous(),
                    remaining_bypasses,
                );
                let request = staged
                    .remove(index)
                    .ok_or_else(|| io::Error::other("scheduler selected an invalid index"))?;
                let device_id = self
                    .device
                    .try_submit(request.operation, &mut trace)
                    .map_err(|error| match error {
                        SubmitError::Rejected(error) => error,
                        SubmitError::Full(_) => io::Error::other("scheduler exceeded device depth"),
                    })?;
                let kind = request.request.kind;
                remaining_bypasses = policy::after_dispatch(remaining_bypasses, index, kind);
                dispatched.push_back((device_id, request.id, kind));
            }

            let (device_id, id, kind) =
                dispatched.pop_front().expect("at least one device request");
            let completion = self
                .device
                .complete_next(Driver::Explicit, &mut trace)?
                .ok_or_else(|| io::Error::other("missing device completion"))?;
            if completion.id != device_id {
                return Err(io::Error::other("unexpected FIFO device completion ID"));
            }
            let wait_ticks = elapsed;
            elapsed += match kind {
                Kind::Read => 2,
                Kind::Write => 8,
                Kind::Barrier => 4,
            };
            finished.push(Finished {
                id,
                response: completion.result?,
                wait_ticks,
                completed_at: elapsed,
            });
        }
        Ok(finished)
    }
}

fn describe(operation: &Operation) -> io::Result<DeviceRequest> {
    let (kind, lba, blocks) = match operation {
        Operation::Read { lba, blocks } => (Kind::Read, *lba, *blocks),
        Operation::Write { lba, data } => (Kind::Write, *lba, data.len()),
        Operation::Flush => (Kind::Barrier, 0, 0),
    };
    let range_error =
        || io::Error::new(io::ErrorKind::InvalidInput, "request sector range overflow");
    let sectors_per_block = BLOCK_BYTES / 512;
    let sector = lba
        .checked_mul(sectors_per_block)
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(range_error)?;
    let sectors = blocks
        .checked_mul(sectors_per_block)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(range_error)?;
    Ok(DeviceRequest {
        kind,
        sector,
        sectors,
    })
}

fn workload() -> [Operation; 8] {
    let write = |byte| Operation::Write {
        lba: 0,
        data: vec![Box::new([byte; BLOCK_BYTES])],
    };
    let read = |lba| Operation::Read { lba, blocks: 1 };
    [
        write(0x12),
        read(0),
        read(1),
        read(2),
        Operation::Flush,
        write(0x34),
        read(3),
        read(0),
    ]
}

fn verify(device: HostScheduler, finished: Vec<Finished>) -> io::Result<()> {
    for completion in finished {
        let expected = match completion.id {
            2 => Some(0x12),
            3 | 4 | 7 => Some(0),
            8 => Some(0x34),
            _ => None,
        };
        match (expected, completion.response) {
            (None, Response::Done) => {}
            (Some(byte), Response::Read(blocks))
                if blocks.len() == 1 && blocks[0].iter().all(|&value| value == byte) => {}
            _ => return Err(io::Error::other("request completed with unexpected data")),
        }
    }
    let mut trace = Trace::new();
    let reopened = Ftl::open(device.power_cut(), &mut trace)?;
    if !reopened
        .read(0, &mut trace)?
        .iter()
        .all(|&byte| byte == 0x12)
    {
        return Err(io::Error::other(
            "flush did not preserve the expected write",
        ));
    }
    Ok(())
}

pub fn compare() -> io::Result<()> {
    let depth = NonZeroUsize::new(2).unwrap();
    println!("All requests arrive at tick 0. Device depth 2. Service costs in ticks: read 2, write 8, flush 4.");
    for selection in [Policy::PassThrough, Policy::BoundedReadFirst] {
        let mut device = HostScheduler::new(Ftl::new(), depth, selection);
        let finished = device.execute(workload())?;
        println!("\n{selection:?}");
        println!("request\twait ticks\tcompletion tick");
        for completion in &finished {
            println!(
                "{}\t{}\t{}",
                completion.id, completion.wait_ticks, completion.completed_at
            );
        }
        verify(device, finished)?;
        println!("Verified all read data and recovered the write before the flush.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_reads_advance_without_crossing_a_write_or_flush_dependency() {
        for depth in [NonZeroUsize::MIN, NonZeroUsize::new(2).unwrap()] {
            let mut fifo = HostScheduler::new(Ftl::new(), depth, Policy::PassThrough);
            let mut read_first = HostScheduler::new(Ftl::new(), depth, Policy::BoundedReadFirst);
            let baseline = fifo.execute(workload()).unwrap();
            let scheduled = read_first.execute(workload()).unwrap();
            let order = |run: &[Finished]| run.iter().map(|result| result.id).collect::<Vec<_>>();
            assert_eq!(order(&baseline), [1, 2, 3, 4, 5, 6, 7, 8]);
            assert_eq!(
                order(&scheduled),
                [3, 1, 2, 4, 5, 7, 6, 8],
                "advance one disjoint read, then serve the older write"
            );
            assert_eq!(scheduled[0].wait_ticks, 0);
            assert_eq!(baseline[2].wait_ticks, 10);
            assert_eq!(
                scheduled.last().unwrap().completed_at,
                baseline.last().unwrap().completed_at
            );
            verify(fifo, baseline).unwrap();
            verify(read_first, scheduled).unwrap();
        }
    }
    #[test]
    fn failure_stops_the_scheduler_before_it_can_reuse_pending_work() {
        let depth = NonZeroUsize::new(2).unwrap();
        let mut scheduler = HostScheduler::new(Ftl::new(), depth, Policy::PassThrough);
        let invalid = Operation::Read {
            lba: scheduler.logical_blocks(),
            blocks: 1,
        };
        let batch = [Operation::Flush, invalid];
        let failure = scheduler.execute(batch).err().unwrap();
        assert_eq!(failure.kind(), io::ErrorKind::InvalidInput);
        let retry = scheduler.execute([Operation::Flush]).err().unwrap();
        assert_eq!(retry.kind(), io::ErrorKind::BrokenPipe);
    }
}
