use std::io;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

use io_journey::ftl::{FlashImage, Ftl};
use io_journey::nand::{Block, NandGeometry, BLOCK_BYTES};
use io_journey::queue::{BlockQueue, Driver, Operation, Response, SubmitError};
use io_journey::Trace;
use libublk::helpers::IoBuf;
use libublk::io::{UblkDev, UblkIOCtx, UblkQueue};
use libublk::{sys, BufDesc, BufDescList, UblkError, UblkIORes};

use crate::profile::{Profile, Timing};
use crate::schedule::{next_index, Arrival, Policy};

pub const DEVICE_BYTES: usize = 32 * 1024 * 1024;
pub const QUEUE_DEPTH: u16 = 8;
pub const IO_BUFFER_BYTES: u32 = 128 * 1024;
const SECTORS_PER_BLOCK: u64 = (BLOCK_BYTES / 512) as u64;

#[derive(Clone, Copy, Default)]
pub struct Counts {
    pub reads: u64,
    pub writes: u64,
    pub flushes: u64,
}

pub struct LabDevice {
    queue: BlockQueue,
    trace: Trace,
    counts: Counts,
    saved: Option<(Counts, FlashImage)>,
    profile: Option<Profile>,
}

impl LabDevice {
    pub fn new(profile: Option<PathBuf>) -> io::Result<Self> {
        let geometry = NandGeometry::new(160, 64)?;
        let ftl = Ftl::with_geometry(DEVICE_BYTES / BLOCK_BYTES, geometry)?;
        Ok(Self {
            queue: BlockQueue::new(ftl, NonZeroUsize::MIN),
            trace: Vec::new(),
            counts: Counts::default(),
            saved: None,
            profile: profile.map(Profile::new),
        })
    }

    pub fn start_profile(&mut self) -> io::Result<()> {
        self.profile
            .as_mut()
            .ok_or_else(|| io::Error::other("server was started without profiling"))?
            .start();
        Ok(())
    }

    pub fn save_profile(&mut self) -> io::Result<usize> {
        self.profile
            .as_mut()
            .ok_or_else(|| io::Error::other("server was started without profiling"))?
            .save()
    }

    pub fn counts(&self) -> Counts {
        self.counts
    }

    pub fn save_checkpoint(&mut self) -> Counts {
        self.saved = Some((self.counts, self.queue.checkpoint()));
        self.counts
    }

    pub fn cut(&mut self) -> io::Result<Counts> {
        let (counts, image) = self
            .saved
            .take()
            .ok_or_else(|| io::Error::other("capture the workload checkpoint before cutting"))?;
        self.trace.clear();
        self.queue = BlockQueue::new(Ftl::open(image, &mut self.trace)?, NonZeroUsize::MIN);
        eprintln!("reopened checkpoint at flush {}", counts.flushes);
        Ok(counts)
    }

    fn handle(
        &mut self,
        tag: u16,
        iod: &sys::ublksrv_io_desc,
        buffer: &mut [u8],
    ) -> io::Result<i32> {
        self.trace.clear();
        let op = iod.op_flags & 0xff;
        if op == sys::UBLK_IO_OP_FLUSH {
            let (id, _) = self.execute(Operation::Flush)?;
            self.counts.flushes += 1;
            eprintln!(
                "ublk tag {tag} request {id} flush {} published FTL checkpoint",
                self.counts.flushes
            );
            return Ok(0);
        }

        if op != sys::UBLK_IO_OP_READ && op != sys::UBLK_IO_OP_WRITE {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "unsupported ublk operation",
            ));
        }
        // blk-mq supplies a separate flush for FUA when native FUA is disabled.
        if iod.op_flags & sys::UBLK_IO_F_FUA != 0 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "unexpected native FUA",
            ));
        }
        let (lba, blocks) = block_span(
            iod.start_sector,
            iod.nr_sectors,
            self.queue.logical_blocks(),
        )?;
        let bytes = blocks * BLOCK_BYTES;
        let payload = buffer
            .get_mut(..bytes)
            .ok_or_else(|| invalid("request exceeds its buffer"))?;

        if op == sys::UBLK_IO_OP_READ {
            let (_, Response::Read(data)) = self.execute(Operation::Read { lba, blocks })? else {
                return Err(io::Error::other("read completed without data"));
            };
            if data.len() != blocks {
                return Err(io::Error::other("short read from the simulator"));
            }
            for (destination, block) in payload.chunks_exact_mut(BLOCK_BYTES).zip(data) {
                destination.copy_from_slice(block.as_slice());
            }
            self.counts.reads += 1;
        } else {
            let data = payload
                .chunks_exact(BLOCK_BYTES)
                .map(|block| Box::new(block.try_into().expect("whole block")))
                .collect::<Vec<Block>>();
            self.execute(Operation::Write { lba, data })?;
            #[cfg(feature = "learner")]
            if self.counts.writes == 0 {
                if let Some(event) = self
                    .trace
                    .iter()
                    .find(|event| event.starts_with("ftl    map LBA "))
                {
                    eprintln!("learner {event}");
                }
            }
            self.counts.writes += 1;
        }
        Ok(bytes as i32)
    }

    fn execute(&mut self, op: Operation) -> io::Result<(u64, Response)> {
        let id = self
            .queue
            .try_submit(op, &mut self.trace)
            .map_err(|error| match error {
                SubmitError::Rejected(error) => error,
                SubmitError::Full(_) => io::Error::other("ublk queue has an unfinished request"),
            })?;
        let completion = self
            .queue
            .complete_next(Driver::Explicit, &mut self.trace)?
            .ok_or_else(|| io::Error::other("ublk request has no completion"))?;
        if completion.id != id {
            return Err(io::Error::other("unexpected completion ID"));
        }
        Ok((id, completion.result?))
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn block_span(sector: u64, sectors: u32, capacity: usize) -> io::Result<(usize, usize)> {
    if sector % SECTORS_PER_BLOCK != 0
        || u64::from(sectors) % SECTORS_PER_BLOCK != 0
        || sectors == 0
    {
        return Err(invalid("ublk request must use whole 4 KiB blocks"));
    }
    let lba = usize::try_from(sector / SECTORS_PER_BLOCK).map_err(|_| invalid("LBA overflows"))?;
    let blocks = usize::try_from(u64::from(sectors) / SECTORS_PER_BLOCK)
        .map_err(|_| invalid("block count overflows"))?;
    if lba.checked_add(blocks).is_none_or(|end| end > capacity) {
        return Err(invalid("ublk request crosses the end of the device"));
    }
    Ok((lba, blocks))
}

pub fn run_queue(
    qid: u16,
    dev: &UblkDev,
    state: &Mutex<LabDevice>,
    policy: Policy,
) -> Result<(), UblkError> {
    let mut buffers = dev.alloc_queue_io_bufs();
    let queue = UblkQueue::new(qid, dev)?
        .submit_fetch_commands_unified(BufDescList::Slices(Some(&buffers)))?;
    let mut batch = Vec::with_capacity(usize::from(QUEUE_DEPTH));
    let outcome = loop {
        // This target submits only ublk fetch and commit commands. The raw
        // callback also sees aborts that the library's IO callback filters.
        let result = queue.flush_and_wake_io_tasks(
            |user_data, cqe, is_last| {
                visit_command(
                    &mut batch,
                    cqe.result(),
                    is_last,
                    || {
                        let tag = UblkIOCtx::user_data_to_tag(user_data) as u16;
                        assert!(queue.owns_tag(tag));
                        let iod = queue.get_iod(tag);
                        Arrival {
                            tag,
                            op: iod.op_flags & 0xff,
                            sector: iod.start_sector,
                            sectors: iod.nr_sectors,
                            at: Instant::now(),
                        }
                    },
                    |pending| service_batch(&queue, &mut buffers, state, policy, pending),
                );
            },
            1,
        );
        match result {
            Ok(_) => {}
            Err(UblkError::QueueIsDown) => break Ok(()),
            Err(error) => break Err(error),
        }
    };
    for tag in queue.tags() {
        queue.unregister_io_buf(tag);
    }
    outcome
}

fn visit_command(
    batch: &mut Vec<Arrival>,
    result: i32,
    is_last: bool,
    describe: impl FnOnce() -> Arrival,
    dispatch: impl FnOnce(&mut Vec<Arrival>),
) {
    if result == sys::UBLK_IO_RES_OK as i32 {
        batch.push(describe());
    }
    // libublk marks a batch containing an abort as stopping after the last
    // callback. Commits queued after that update would be dropped.
    if is_last {
        dispatch(batch);
    }
}

fn service_batch(
    queue: &UblkQueue,
    buffers: &mut [IoBuf<u8>],
    state: &Mutex<LabDevice>,
    policy: Policy,
    batch: &mut Vec<Arrival>,
) {
    while !batch.is_empty() {
        let index = next_index(policy, batch);
        let arrival = batch.remove(index);
        let tag = arrival.tag;
        let buffer = &mut buffers[usize::from(tag)];
        let iod = queue.get_iod(tag);
        let result = match state.lock() {
            Ok(mut device) => {
                let device = &mut *device;
                let started = Instant::now();
                let result = linux_result(tag, device.handle(tag, iod, buffer.as_mut_slice()));
                let service = started.elapsed();
                if let Some(profile) = &mut device.profile {
                    profile.record(
                        Timing {
                            tag,
                            operation: arrival.operation(),
                            sector: arrival.sector,
                            sectors: arrival.sectors,
                            wait_ns: started.duration_since(arrival.at).as_nanos(),
                            service_ns: service.as_nanos(),
                            result,
                            advanced: index != 0,
                        },
                        &device.trace,
                    );
                }
                result
            }
            Err(_) => linux_result(tag, Err(io::Error::other("device state lock is poisoned"))),
        };
        if let Err(error) = queue.complete_io_cmd_unified(
            tag,
            BufDesc::Slice(buffer.as_slice()),
            Ok(UblkIORes::Result(result)),
        ) {
            eprintln!("could not complete ublk tag {tag}: {error}");
        }
    }
}

fn linux_result(tag: u16, result: io::Result<i32>) -> i32 {
    match result {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("ublk tag {tag} failed: {error}");
            match error.kind() {
                io::ErrorKind::InvalidInput => -libc::EINVAL,
                io::ErrorKind::Unsupported => -libc::EOPNOTSUPP,
                _ => -libc::EIO,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sector_requests_are_aligned_and_bounded() {
        assert_eq!(block_span(8, 16, 3).unwrap(), (1, 2));
        for (sector, sectors) in [(1, 8), (0, 0), (0, 1), (16, 16), (u64::MAX - 7, 8)] {
            assert_eq!(
                block_span(sector, sectors, 3).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }

    #[test]
    fn a_skipped_final_completion_queues_the_accepted_tag_before_stop() {
        let mut batch = Vec::new();
        let mut queued_commits = Vec::new();
        let mut stopping = false;
        for (result, is_last) in [
            (sys::UBLK_IO_RES_OK as i32, false),
            (sys::UBLK_IO_RES_ABORT, true),
        ] {
            visit_command(
                &mut batch,
                result,
                is_last,
                || {
                    assert_eq!(result, sys::UBLK_IO_RES_OK as i32);
                    Arrival {
                        tag: 3,
                        op: sys::UBLK_IO_OP_READ,
                        sector: 0,
                        sectors: 8,
                        at: Instant::now(),
                    }
                },
                |pending| {
                    assert!(is_last);
                    if !stopping {
                        queued_commits.extend(pending.drain(..).map(|request| request.tag));
                    }
                },
            );
            if result == sys::UBLK_IO_RES_ABORT && is_last {
                // Match libublk: it applies the abort after visiting the entries.
                stopping = true;
            }
        }
        assert!(stopping);
        assert_eq!(queued_commits, [3]);
        assert!(batch.is_empty());
    }
}
