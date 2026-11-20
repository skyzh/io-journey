#[cfg_attr(
    feature = "learner-append",
    path = "../../../course/learner/append-commit.rs"
)]
#[cfg_attr(not(feature = "learner-append"), path = "commit/reference.rs")]
mod commit;
use commit::commit_group;

mod engine;

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use io_journey::record;

use crate::engine::{Engine, Request, DEPTH};

const WARM_RECORDS: usize = 64;
const MEASURED_RECORDS: usize = 256;
const COMMITTED_RECORDS: usize = WARM_RECORDS + MEASURED_RECORDS;
const TAIL_RECORDS: usize = 4;
const RECORD_BYTES: usize = 512;
const FILE_NAME: &str = "events.log";

fn main() -> io::Result<()> {
    let args = env::args_os().skip(1).collect::<Vec<_>>();
    match args.as_slice() {
        [action, engine, group, mount] if action == "run" => {
            let engine = engine.to_str().ok_or_else(|| invalid("invalid engine"))?;
            let group = group
                .to_str()
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|&value| matches!(value, 4 | 32))
                .ok_or_else(|| invalid("use group size 4 or 32"))?;
            run(engine, group, Path::new(mount))
        }
        [action, mount] if action == "tail" => unacknowledged_tail(Path::new(mount)),
        [action, mount] if action == "verify" => verify(Path::new(mount), COMMITTED_RECORDS),
        [action, mount, acknowledged] if action == "verify" => {
            let acknowledged = acknowledged
                .to_str()
                .and_then(|value| value.parse::<usize>().ok())
                .ok_or_else(|| invalid("invalid acknowledged record count"))?;
            verify(Path::new(mount), acknowledged)
        }
        _ => Err(invalid(
            "use run sync|uring 4|32 MOUNT, tail MOUNT, or verify MOUNT [COUNT]",
        )),
    }
}

fn run(mode: &str, group: usize, directory: &Path) -> io::Result<()> {
    let records = requests(0, COMMITTED_RECORDS)?;
    let file = Arc::new(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join(FILE_NAME))?,
    );
    // Publish creation before either measured engine starts appending.
    File::open(directory)?.sync_all()?;
    let mut engine = Engine::new(mode, file.clone())?;
    let mut write = |wave: &[Request]| engine.write_wave(wave);
    let mut sync = || file.sync_all();
    let mut warm = Vec::with_capacity(WARM_RECORDS);
    for cohort in records[..WARM_RECORDS].chunks(group) {
        commit_group(cohort, &mut write, &mut sync, &mut warm)?;
    }
    if warm.len() != WARM_RECORDS {
        return Err(io::Error::other("warmup did not acknowledge every record"));
    }

    let mut receipts = Vec::with_capacity(MEASURED_RECORDS);
    let start = Instant::now();
    for cohort in records[WARM_RECORDS..].chunks(group) {
        if let Err(error) = commit_group(cohort, &mut write, &mut sync, &mut receipts) {
            return Err(io::Error::new(
                error.kind(),
                format!(
                    "stopped with {} of {MEASURED_RECORDS} measured records acknowledged: {error}",
                    receipts.len()
                ),
            ));
        }
    }
    let elapsed = start.elapsed();
    if receipts.len() != MEASURED_RECORDS {
        return Err(io::Error::other(
            "measurement did not acknowledge every record",
        ));
    }
    receipts.sort_unstable();
    let records_per_second = MEASURED_RECORDS as f64 / elapsed.as_secs_f64();
    println!(
        "{mode}\t{group}\t{DEPTH}\t{}\t{}\t{}\t{}\t{records_per_second:.2}\t{}\t{}",
        warm.len(),
        receipts.len(),
        receipts.len() * RECORD_BYTES,
        elapsed.as_nanos(),
        percentile(&receipts, 50).as_nanos(),
        percentile(&receipts, 99).as_nanos()
    );
    Ok(())
}

fn percentile(sorted: &[Duration], percent: usize) -> Duration {
    sorted[(sorted.len() * percent).div_ceil(100) - 1]
}

fn payload(index: usize) -> [u8; RECORD_BYTES - record::HEADER_BYTES] {
    let mut payload = [index as u8; RECORD_BYTES - record::HEADER_BYTES];
    payload[..8].copy_from_slice(&(index as u64).to_le_bytes());
    payload
}

fn requests(start: usize, end: usize) -> io::Result<Vec<Request>> {
    (start..end)
        .map(|index| {
            Ok(Request {
                id: index as u64,
                offset: (index * RECORD_BYTES) as u64,
                bytes: record::encode(&payload(index))?.into(),
            })
        })
        .collect()
}

fn unacknowledged_tail(directory: &Path) -> io::Result<()> {
    let file = OpenOptions::new()
        .write(true)
        .open(directory.join(FILE_NAME))?;
    for request in requests(COMMITTED_RECORDS, COMMITTED_RECORDS + TAIL_RECORDS)? {
        file.write_all_at(&request.bytes, request.offset)?;
    }
    verify(directory, COMMITTED_RECORDS + TAIL_RECORDS)
}

fn verify(directory: &Path, expected: usize) -> io::Result<()> {
    let bytes = fs::read(directory.join(FILE_NAME))?;
    let acknowledged_bytes = expected
        .checked_mul(RECORD_BYTES)
        .ok_or_else(|| invalid("acknowledged byte count overflows"))?;
    if bytes.len() != acknowledged_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "expected {acknowledged_bytes} bytes after reopen, found {}",
                bytes.len()
            ),
        ));
    }
    let records = record::decode_acknowledged(&bytes, acknowledged_bytes)?;
    if records.len() != expected
        || records
            .iter()
            .enumerate()
            .any(|(index, record)| **record != payload(index))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "expected exact CRC-checked prefix of {expected} records, found {}",
                records.len()
            ),
        ));
    }
    Ok(())
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn a_group_syncs_once_after_all_waves() {
        let requests = requests(0, 3 * DEPTH).unwrap();
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
        let requests = requests(0, 12).unwrap();
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
        let requests = requests(0, 4).unwrap();
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
}
