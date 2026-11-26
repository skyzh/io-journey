//! Shared completion validation and a portable model of two bounded rings.
use std::collections::VecDeque;
use std::io;
use std::sync::Arc;

#[cfg_attr(
    feature = "learner-append",
    path = "../course/learner/append-completions.rs"
)]
#[cfg_attr(not(feature = "learner-append"), path = "ring/reference.rs")]
mod completion;
pub use completion::{check_completions, drain_ring_wave};

#[cfg_attr(
    feature = "learner-append",
    path = "../course/learner/append-commit.rs"
)]
#[cfg_attr(not(feature = "learner-append"), path = "ring/commit.rs")]
mod commit;
pub use commit::commit_group;

pub const DEPTH: usize = 4;

#[derive(Clone)]
pub struct Request {
    pub id: u64,
    pub offset: u64,
    pub bytes: Arc<[u8]>,
}

pub struct Completion {
    pub id: u64,
    pub result: io::Result<usize>,
}

#[cfg(not(feature = "learner-append"))]
fn invalid_completion(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

struct MockRing {
    capacity: usize,
    submissions: VecDeque<Request>,
    issued: Vec<Request>,
    completions: VecDeque<Completion>,
}

impl MockRing {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            submissions: VecDeque::with_capacity(capacity),
            issued: Vec::with_capacity(capacity),
            completions: VecDeque::with_capacity(capacity),
        }
    }

    fn submit(&mut self, request: Request) -> Result<(), Request> {
        if self.submissions.len() == self.capacity {
            return Err(request);
        }
        self.submissions.push_back(request);
        Ok(())
    }

    fn step(&mut self, write: impl FnOnce(&Request) -> io::Result<usize>) -> bool {
        if self.completions.len() == self.capacity {
            return false;
        }
        // Fetch submissions in order. The mock completes issued work in
        // reverse order to exercise the caller's completion validation.
        let count = (self.capacity - self.issued.len()).min(self.submissions.len());
        self.issued.extend(self.submissions.drain(..count));
        let Some(request) = self.issued.pop() else {
            return false;
        };
        self.completions.push_back(Completion {
            id: request.id,
            result: write(&request),
        });
        true
    }

    fn poll(
        &mut self,
        write: impl FnOnce(&Request) -> io::Result<usize>,
    ) -> io::Result<Completion> {
        if self.completions.is_empty() {
            self.step(write);
        }
        self.completions
            .pop_front()
            .ok_or_else(|| io::Error::other("mock ring has no completion to poll"))
    }
}

fn request(id: u64, offset: u64, bytes: [u8; 3]) -> Request {
    Request {
        id,
        offset,
        bytes: Arc::new(bytes),
    }
}

/// Submit a wave of positional writes and validate each completion.
/// Use disjoint writes when the outcome must be independent of completion order.
pub fn mock_write_wave(
    wave: &[Request],
    mut write: impl FnMut(&Request) -> io::Result<usize>,
) -> io::Result<()> {
    if wave.is_empty() || wave.len() > DEPTH {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "mock write wave must fit the submission queue",
        ));
    }
    let mut ring = MockRing::new(wave.len());
    for request in wave {
        ring.submit(request.clone())
            .map_err(|_| io::Error::other("mock submission queue is full"))?;
    }
    drain_ring_wave(wave, |completions| {
        completions.push(ring.poll(&mut write)?);
        Ok(())
    })
}

pub fn demo() -> io::Result<()> {
    let wave = [request(1, 0, *b"abc"), request(2, 3, *b"def")];
    let mut file = [0; 6];
    for request in &wave {
        println!("submitted {} at byte offset {}", request.id, request.offset);
    }
    mock_write_wave(&wave, |request| {
        let offset = request.offset as usize;
        let end = offset + request.bytes.len();
        file[offset..end].copy_from_slice(&request.bytes);
        println!("completed request {}", request.id);
        Ok(request.bytes.len())
    })?;
    println!("file bytes: {}", String::from_utf8_lossy(&file));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_submission_and_completion_queues_apply_backpressure() {
        let mut ring = MockRing::new(1);
        assert!(ring.submit(request(1, 0, *b"abc")).is_ok());
        let blocked = ring.submit(request(2, 3, *b"def")).err().unwrap();
        assert!(ring.step(|_| Ok(3)));
        assert!(ring.submit(blocked).is_ok());
        assert!(!ring.step(|_| panic!("no space for another completion")));
        assert_eq!(ring.poll(|_| unreachable!()).unwrap().id, 1);
        assert_eq!(ring.poll(|_| Ok(3)).unwrap().id, 2);
    }

    #[test]
    fn mock_wave_checks_out_of_order_completions_and_short_writes() {
        let wave = [request(1, 0, *b"abc"), request(2, 3, *b"def")];
        let run = |short_write| {
            let mut seen = Vec::new();
            let result = mock_write_wave(&wave, |request| {
                seen.push(request.id);
                if short_write && request.id == 2 {
                    Ok(1)
                } else {
                    Ok(3)
                }
            });
            (result, seen)
        };
        let (result, seen) = run(false);
        result.unwrap();
        assert_eq!(seen, [2, 1], "retire the whole wave out of order");
        let (result, seen) = run(true);
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::WriteZero);
        assert_eq!(seen, [2, 1], "retire the whole wave after a short write");
    }
    #[test]
    fn completion_validation_rejects_missing_duplicate_and_unknown_ids() {
        let wave = [request(1, 0, *b"abc"), request(2, 3, *b"def")];
        for ids in [&[1][..], &[1, 1], &[1, 3]] {
            let completed = ids
                .iter()
                .map(|&id| Completion { id, result: Ok(3) })
                .collect();
            let failure = check_completions(&wave, completed).unwrap_err();
            assert_eq!(failure.kind(), io::ErrorKind::InvalidData);
        }
    }
}

#[cfg(test)]
mod commit_tests {
    use super::*;
    use std::cell::Cell;

    fn requests(count: usize) -> Vec<Request> {
        (0..count)
            .map(|id| request(id as u64, 3 * id as u64, *b"abc"))
            .collect()
    }

    #[test]
    fn a_group_syncs_once_after_all_waves() {
        let requests = requests(3 * DEPTH);
        let write_calls = Cell::new(0);
        let sync_calls = Cell::new(0);
        let mut receipts = Vec::new();
        commit_group(
            &requests,
            &mut |wave| {
                assert_eq!(wave.len(), DEPTH);
                assert_eq!(sync_calls.get(), 0);
                write_calls.set(write_calls.get() + 1);
                Ok(())
            },
            &mut || {
                assert_eq!(write_calls.get(), 3);
                sync_calls.set(sync_calls.get() + 1);
                Ok(())
            },
            &mut receipts,
        )
        .unwrap();
        assert_eq!(sync_calls.get(), 1);
        assert_eq!(receipts.len(), requests.len());
    }

    #[test]
    fn short_write_or_error_cannot_acknowledge_or_sync_the_failed_group() {
        let requests = requests(12);
        let mut receipts = Vec::new();
        commit_group(
            &requests[..4],
            &mut |_| Ok(()),
            &mut || Ok(()),
            &mut receipts,
        )
        .unwrap();
        for failure in [io::ErrorKind::WriteZero, io::ErrorKind::PermissionDenied] {
            let result = commit_group(
                &requests[4..],
                &mut |wave| {
                    if wave[0].id == 8 {
                        Err(failure.into())
                    } else {
                        Ok(())
                    }
                },
                &mut || panic!("failed wave reached sync"),
                &mut receipts,
            );
            assert_eq!(result.unwrap_err().kind(), failure);
            assert_eq!(receipts.len(), 4);
        }
    }

    #[test]
    fn sync_error_prevents_a_group_receipt() {
        let requests = requests(4);
        let mut receipts = Vec::new();
        assert!(commit_group(
            &requests,
            &mut |_| Ok(()),
            &mut || Err(io::Error::other("flush failed")),
            &mut receipts
        )
        .is_err());
        assert!(receipts.is_empty());
    }
    #[test]
    fn a_short_mock_write_prevents_sync_and_receipts() {
        let requests = requests(2);
        let mut receipts = Vec::new();
        let failure = commit_group(
            &requests,
            &mut |wave| mock_write_wave(wave, |_| Ok(1)),
            &mut || panic!("short write did not stop the group"),
            &mut receipts,
        )
        .unwrap_err();
        assert_eq!(failure.kind(), io::ErrorKind::WriteZero);
        assert!(receipts.is_empty());
    }
}
