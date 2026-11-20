mod app;
mod filesystem;

use std::io;
use std::num::NonZeroUsize;

use filesystem::{FileSystem, FIRST_DATA_LBA};
use ftl::{Ftl, TornProgram};
use io_journey::record::{DEMO_RECORDS as ACKNOWLEDGED, HEADER_BYTES};
use io_journey::{ftl, nand, queue, Trace};
use nand::ERASED_BYTE;
use queue::BlockQueue;

const QUEUE_DEPTH: NonZeroUsize = NonZeroUsize::new(2).unwrap();
const VOLATILE: &[u8] = b"volatile";
const TORN: &[u8] = b"torn";

fn main() -> io::Result<()> {
    let mut trace = Vec::new();
    let result = demo(&mut trace);
    for event in trace {
        println!("{event}");
    }
    result
}

fn demo(trace: &mut Trace) -> io::Result<()> {
    let mut queue = BlockQueue::new(Ftl::new(), QUEUE_DEPTH);
    {
        let mut fs = FileSystem::format(&mut queue, trace)?;
        for payload in ACKNOWLEDGED {
            app::append(&mut fs, payload, trace)?;
        }
        fs.sync(trace)?;
        trace.push("app    acknowledged 2 records after flush".into());

        app::append(&mut fs, VOLATILE, trace)?;
        verify_records(
            &mut fs,
            &[ACKNOWLEDGED[0], ACKNOWLEDGED[1], VOLATILE],
            "before simulated cut",
            trace,
        )?;
    }

    trace.push("demo   power cut: reopen from the durable RAM image".into());
    let mut queue = BlockQueue::new(Ftl::open(queue.power_cut(), trace)?, QUEUE_DEPTH);
    verify_records(
        &mut FileSystem::open(&mut queue, trace)?,
        &ACKNOWLEDGED,
        "after volatile cut",
        trace,
    )?;

    // The new page would contain the two committed records followed by the
    // new frame. Tear just after writing two bytes of its payload.
    let committed_bytes: usize = ACKNOWLEDGED
        .iter()
        .map(|record| HEADER_BYTES + record.len())
        .sum();
    let tear_after = committed_bytes + HEADER_BYTES + 2;
    queue.arm_torn_write(FIRST_DATA_LBA, tear_after)?;
    let flush_error = {
        let mut fs = FileSystem::open(&mut queue, trace)?;
        app::append(&mut fs, TORN, trace)?;
        match fs.sync(trace) {
            Ok(()) => {
                return Err(io::Error::other(
                    "flush succeeded despite the injected tear",
                ))
            }
            Err(error) => error,
        }
    };
    let Some(tear) = flush_error
        .get_ref()
        .and_then(|cause| cause.downcast_ref::<TornProgram>())
    else {
        return Err(flush_error);
    };
    let image = queue.power_cut();
    let partial = image
        .physical_page(tear.page)
        .ok_or_else(|| io::Error::other("torn page was lost at cut"))?;
    let committed_page = image
        .mapped_page(tear.lba)
        .ok_or_else(|| io::Error::other("durable mapping was lost at cut"))?;
    if committed_page == tear.page
        || partial[tear.bytes - 2..tear.bytes] != TORN[..2]
        || partial[tear.bytes..]
            .iter()
            .any(|&byte| byte != ERASED_BYTE)
    {
        return Err(io::Error::other(
            "torn page contents or mapping were unexpected",
        ));
    }
    trace.push(format!(
        "demo   cut left {} programmed bytes on page {}. LBA {} recovers from page {committed_page}",
        tear.bytes, tear.page, tear.lba
    ));
    let mut queue = BlockQueue::new(Ftl::open(image, trace)?, QUEUE_DEPTH);
    verify_records(
        &mut FileSystem::open(&mut queue, trace)?,
        &ACKNOWLEDGED,
        "after torn program",
        trace,
    )?;
    trace.push("demo   verified the acknowledged prefix after both cuts".into());
    Ok(())
}

fn verify_records(
    fs: &mut FileSystem<'_>,
    expected: &[&[u8]],
    phase: &str,
    trace: &mut Trace,
) -> io::Result<()> {
    let records = app::recover(fs, trace)?;
    if !records
        .iter()
        .map(Vec::as_slice)
        .eq(expected.iter().copied())
    {
        return Err(io::Error::other(format!(
            "unexpected records {phase}: {records:?}"
        )));
    }
    let names: Vec<_> = records
        .iter()
        .map(|bytes| String::from_utf8_lossy(bytes))
        .collect();
    trace.push(format!("app    read {phase}: {names:?}"));
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn demo_recovers_the_acknowledged_prefix_across_both_cuts() {
        super::demo(&mut Vec::new()).unwrap();
    }
}
