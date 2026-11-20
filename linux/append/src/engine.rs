#[cfg_attr(
    feature = "learner-append",
    path = "../../../course/learner/append-completions.rs"
)]
#[cfg_attr(not(feature = "learner-append"), path = "engine/reference.rs")]
mod completion;
use completion::{check_completions, retire_ring_wave};

use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::FileExt;
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};

use io_uring::{opcode, types, IoUring};

pub const DEPTH: usize = 4;

#[derive(Clone)]
pub struct Request {
    pub id: u64,
    pub offset: u64,
    pub bytes: Arc<[u8]>,
}

pub enum Engine {
    Sync(Blocking),
    Ring(Uring),
}

impl Engine {
    pub fn new(mode: &str, file: Arc<File>) -> io::Result<Self> {
        match mode {
            "sync" => Ok(Self::Sync(Blocking::new(file)?)),
            "uring" => Ok(Self::Ring(Uring {
                ring: IoUring::new(DEPTH as u32)?,
                file,
                retained: Vec::with_capacity(DEPTH),
            })),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected sync or uring",
            )),
        }
    }

    pub fn write_wave(&mut self, wave: &[Request]) -> io::Result<()> {
        assert!(!wave.is_empty() && wave.len() <= DEPTH);
        match self {
            Self::Sync(engine) => engine.write_wave(wave),
            Self::Ring(engine) => engine.write_wave(wave),
        }
    }
}

#[cfg_attr(feature = "learner-append", allow(dead_code))]
struct Completion {
    id: u64,
    result: io::Result<usize>,
}

pub struct Blocking {
    requests: Vec<mpsc::SyncSender<Request>>,
    replies: mpsc::Receiver<Completion>,
    workers: Vec<JoinHandle<()>>,
}

impl Blocking {
    fn new(file: Arc<File>) -> io::Result<Self> {
        let (reply, replies) = mpsc::sync_channel(DEPTH);
        let mut engine = Self {
            requests: Vec::new(),
            replies,
            workers: Vec::new(),
        };
        for _ in 0..DEPTH {
            let (sender, requests) = mpsc::sync_channel::<Request>(1);
            let reply = reply.clone();
            let file = file.clone();
            let worker = thread::Builder::new().spawn(move || {
                for request in requests {
                    let result = file.write_at(&request.bytes, request.offset);
                    if reply
                        .send(Completion {
                            id: request.id,
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })?;
            engine.requests.push(sender);
            engine.workers.push(worker);
        }
        Ok(engine)
    }

    fn write_wave(&self, wave: &[Request]) -> io::Result<()> {
        for (request, sender) in wave.iter().zip(&self.requests) {
            sender
                .send(request.clone())
                .map_err(|_| io::Error::other("write worker stopped"))?;
        }
        let mut completed = Vec::with_capacity(wave.len());
        for _ in wave {
            completed.push(
                self.replies
                    .recv()
                    .map_err(|_| io::Error::other("missing worker reply"))?,
            );
        }
        check_completions(wave, completed)
    }
}

impl Drop for Blocking {
    fn drop(&mut self) {
        self.requests.clear();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

pub struct Uring {
    ring: IoUring,
    file: Arc<File>,
    retained: Vec<Arc<[u8]>>,
}

impl Uring {
    fn write_wave(&mut self, wave: &[Request]) -> io::Result<()> {
        if !self.retained.is_empty() {
            return Err(io::Error::other("the previous ring wave did not retire"));
        }
        {
            let mut queue = self.ring.submission();
            if queue.capacity() - queue.len() < wave.len() {
                return Err(io::Error::other("could not reserve the ring wave"));
            }
            for request in wave {
                self.retained.push(request.bytes.clone());
                let entry = opcode::Write::new(
                    types::Fd(self.file.as_raw_fd()),
                    request.bytes.as_ptr(),
                    request.bytes.len() as u32,
                )
                .offset(request.offset)
                .build()
                .user_data(request.id);
                // Keep each immutable allocation and the file descriptor alive
                // until its completion. This ring has one submitter.
                unsafe {
                    queue
                        .push(&entry)
                        .map_err(|_| io::Error::other("ring became full"))?;
                }
            }
        }

        let ring = &mut self.ring;
        retire_ring_wave(wave, &mut self.retained, |completed| {
            loop {
                match ring.submit_and_wait(1) {
                    Ok(_) => break,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(error),
                }
            }
            completed.extend(ring.completion().map(|entry| {
                let result = entry.result();
                let result = if result >= 0 {
                    Ok(result as usize)
                } else {
                    Err(io::Error::from_raw_os_error(-result))
                };
                Completion {
                    id: entry.user_data(),
                    result,
                }
            }));
            Ok(())
        })
    }
}

impl Drop for Uring {
    fn drop(&mut self) {
        if !self.retained.is_empty() {
            // A failed wait or an invalid completion set cannot prove the
            // kernel has stopped using these pointers. Keep at most one
            // wave alive until this CLI process exits.
            std::mem::forget(std::mem::take(&mut self.retained));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(id: u64) -> Request {
        Request {
            id,
            offset: id * 512,
            bytes: vec![0; 512].into(),
        }
    }

    fn done(id: u64, bytes: usize) -> Completion {
        Completion {
            id,
            result: Ok(bytes),
        }
    }

    #[test]
    fn completion_id_and_length_are_checked_even_when_results_reorder() {
        let wave = [request(5), request(6)];
        check_completions(&wave, vec![done(6, 512), done(5, 512)]).unwrap();
        for invalid in [
            vec![done(5, 512)],
            vec![done(5, 512), done(5, 512)],
            vec![done(5, 512), done(7, 512)],
        ] {
            assert_eq!(
                check_completions(&wave, invalid).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
        let short = check_completions(&wave, vec![done(6, 511), done(5, 512)]).unwrap_err();
        assert_eq!(short.kind(), io::ErrorKind::WriteZero);
        let bad = Completion {
            id: 5,
            result: Err(io::ErrorKind::PermissionDenied.into()),
        };
        assert_eq!(
            check_completions(&wave, vec![bad, done(6, 512)])
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
    }

    #[test]
    fn early_short_or_failed_completion_keeps_other_buffers_until_their_replies() {
        for failure in [io::ErrorKind::WriteZero, io::ErrorKind::PermissionDenied] {
            let wave = [request(5), request(6)];
            let mut retained = wave.iter().map(|item| item.bytes.clone()).collect();
            let mut polls = 0;
            let error = retire_ring_wave(&wave, &mut retained, |completed| {
                assert_eq!(Arc::strong_count(&wave[1].bytes), 2);
                if polls == 0 {
                    completed.push(if failure == io::ErrorKind::WriteZero {
                        done(5, 511)
                    } else {
                        Completion {
                            id: 5,
                            result: Err(failure.into()),
                        }
                    });
                } else {
                    completed.push(done(6, 512));
                }
                polls += 1;
                Ok(())
            })
            .unwrap_err();
            assert_eq!(polls, 2);
            assert_eq!(error.kind(), failure);
            assert!(retained.is_empty());
            assert_eq!(Arc::strong_count(&wave[1].bytes), 1);
        }
    }
}
