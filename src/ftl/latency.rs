//! An open-loop timing exercise using the reference FTL's physical operations.

use std::io;
use std::num::NonZeroU32;

use super::{Ftl, NandGeometry, BLOCK_BYTES};
use crate::nand::PhysicalOp;

const PAGES_PER_BLOCK: usize = 4;
const LOGICAL_PAGES: usize = 12;
const TICKS: u64 = 24;
const ARRIVAL_INTERVAL_US: u64 = 200;
const READ_OFFSET_US: u64 = 25;
const COLD_LBAS: [usize; 2] = [0, 4];
const HOT_LBA: usize = 8;

#[derive(Clone, Copy)]
pub struct ServiceTimes {
    pub read: NonZeroU32,
    pub program: NonZeroU32,
    pub erase: NonZeroU32,
}

impl Default for ServiceTimes {
    fn default() -> Self {
        Self {
            read: NonZeroU32::new(80).unwrap(),
            program: NonZeroU32::new(400).unwrap(),
            erase: NonZeroU32::new(1600).unwrap(),
        }
    }
}

impl ServiceTimes {
    fn duration(self, operation: PhysicalOp) -> u64 {
        u64::from(
            match operation {
                PhysicalOp::Read(_) => self.read,
                PhysicalOp::Program(_) => self.program,
                PhysicalOp::Erase(_) => self.erase,
            }
            .get(),
        )
    }
}

struct Clock {
    ready: [u64; 2],
    times: ServiceTimes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Service {
    operation: PhysicalOp,
    die: usize,
    start: u64,
    finish: u64,
}

impl Clock {
    fn schedule(&mut self, operation: PhysicalOp, ready: u64) -> Service {
        let block = match operation {
            PhysicalOp::Read(page) | PhysicalOp::Program(page) => page / PAGES_PER_BLOCK,
            PhysicalOp::Erase(block) => block,
        };
        // One cold block has its own die. All writable spare space and the
        // hot LBA use die 0, even when the larger arm adds spare blocks.
        let die = usize::from(block == 1);
        let start = ready.max(self.ready[die]);
        let finish = start + self.times.duration(operation);
        self.ready[die] = finish;
        Service {
            operation,
            die,
            start,
            finish,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Read {
    arrival: u64,
    lba: usize,
    service: Service,
}

#[derive(Default)]
struct Trial {
    reads: Vec<Read>,
    host_programs: usize,
    gc_copies: usize,
    gc_programs: usize,
    gc_erases: usize,
    first_collection: Option<(u64, Vec<Service>)>,
}

impl Trial {
    fn elapsed(&self) -> u64 {
        self.reads
            .iter()
            .map(|read| read.service.finish)
            .max()
            .unwrap()
            - self.reads[0].arrival
    }

    fn p99(&self) -> u64 {
        let mut latencies: Vec<_> = self
            .reads
            .iter()
            .map(|read| read.service.finish - read.arrival)
            .collect();
        latencies.sort_unstable();
        latencies[(latencies.len() * 99).div_ceil(100) - 1]
    }
}

fn replay(times: ServiceTimes, erase_blocks: usize, writes: bool) -> io::Result<Trial> {
    let geometry = NandGeometry::new(erase_blocks, PAGES_PER_BLOCK)?;
    let mut ftl = Ftl::with_geometry(LOGICAL_PAGES, geometry)?;
    let mut trace = Vec::new();
    for lba in 0..LOGICAL_PAGES {
        ftl.write(lba, Box::new([lba as u8; BLOCK_BYTES]), &mut trace)?;
    }
    let mut clock = Clock {
        ready: [0; 2],
        times,
    };
    let mut trial = Trial::default();

    for tick in 0..TICKS {
        trace.clear();
        let arrival = tick * ARRIVAL_INTERVAL_US;
        if writes {
            // Plan the write using the real FTL, then charge its operations
            // on the virtual clock. Foreground reads address unchanged data.
            ftl.live.nand.start_capture();
            let written = ftl.write(HOT_LBA, Box::new([tick as u8; BLOCK_BYTES]), &mut trace);
            let operations = ftl.live.nand.finish_capture();
            written?;
            let Some((&PhysicalOp::Program(host_page), collection)) = operations.split_last()
            else {
                return Err(io::Error::other("FTL write did not end in a host program"));
            };
            let mut ready = arrival;
            let mut services = Vec::new();
            for &operation in collection {
                match operation {
                    PhysicalOp::Read(_) => trial.gc_copies += 1,
                    PhysicalOp::Program(_) => trial.gc_programs += 1,
                    PhysicalOp::Erase(_) => trial.gc_erases += 1,
                }
                let service = clock.schedule(operation, ready);
                ready = service.finish;
                services.push(service);
            }
            if !services.is_empty() && trial.first_collection.is_none() {
                trial.first_collection = Some((arrival, services));
            }
            clock.schedule(PhysicalOp::Program(host_page), ready);
            trial.host_programs += 1;
        }

        let arrival = arrival + READ_OFFSET_US;
        for lba in COLD_LBAS {
            let page = ftl.live.mapped_page(lba).expect("preconditioned cold page");
            let value = ftl.read(lba, &mut trace)?;
            if page != lba || value.iter().any(|&byte| byte != lba as u8) {
                return Err(io::Error::other("cold read moved or changed during replay"));
            }
            let service = clock.schedule(PhysicalOp::Read(page), arrival);
            trial.reads.push(Read {
                arrival,
                lba,
                service,
            });
        }
    }
    Ok(trial)
}

pub fn run(times: ServiceTimes) -> io::Result<()> {
    println!(
        "Virtual NAND, illustrative service times (us): read {}, program {}, erase {}",
        times.read, times.program, times.erase
    );
    println!(
        "{TICKS} ticks spaced {ARRIVAL_INTERVAL_US} us apart: optional write to LBA {HOT_LBA}, then          two 4 KiB reads to LBAs 0 and 4 at +{READ_OFFSET_US} us"
    );
    println!("Throughput window: first offered read to last completed read, including drain");
    println!(
        "{:<21} {:>5} {:>8} {:>10} {:>10} {:>11} {:>8} {:>8} {:>8} {:>8}",
        "workload",
        "reads",
        "bytes",
        "window us",
        "read MiB/s",
        "read p99 us",
        "host pgm",
        "GC copy",
        "GC pgm",
        "erase"
    );
    for (name, erase_blocks, writes) in [
        ("Read only", 10, false),
        ("Mixed, spare pages", 10, true),
        ("Mixed, collecting", 5, true),
    ] {
        let trial = replay(times, erase_blocks, writes)?;
        let bytes = trial.reads.len() * BLOCK_BYTES;
        let elapsed = trial.elapsed();
        let mib_per_sec = bytes as f64 * 1_000_000.0 / (elapsed as f64 * 1_048_576.0);
        println!(
            "{name:<21} {:>5} {bytes:>8} {elapsed:>10} {mib_per_sec:>10.2} {:>11} {:>8} {:>8} {:>8} {:>8}",
            trial.reads.len(), trial.p99(), trial.host_programs, trial.gc_copies,
            trial.gc_programs, trial.gc_erases
        );
        if let Some((arrival, services)) = trial.first_collection {
            println!("First collection (write offered at {arrival} us):");
            for service in services {
                println!(
                    "  die {}: {:?} at {}..{} us",
                    service.die, service.operation, service.start, service.finish
                );
            }
            for read in trial
                .reads
                .iter()
                .filter(|read| read.arrival == arrival + READ_OFFSET_US)
            {
                println!(
                    "  LBA {} on die {}: read arrived at {}, ran {}..{} us",
                    read.lba,
                    read.service.die,
                    read.arrival,
                    read.service.start,
                    read.service.finish
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_waits_for_a_program_on_its_die_while_the_other_die_reads() {
        let mut clock = Clock {
            ready: [0; 2],
            times: ServiceTimes::default(),
        };
        let program = clock.schedule(PhysicalOp::Program(8), 0);
        let same_die = clock.schedule(PhysicalOp::Read(0), 25);
        let other_die = clock.schedule(PhysicalOp::Read(4), 25);
        assert_eq!((program.start, program.finish), (0, 400));
        assert_eq!((same_die.start, same_die.finish), (400, 480));
        assert_eq!((other_die.start, other_die.finish), (25, 105));
    }

    #[test]
    fn same_reads_and_writes_charge_only_actual_ftl_collection_work() {
        let times = ServiceTimes::default();
        let read_only = replay(times, 10, false).unwrap();
        let spare = replay(times, 10, true).unwrap();
        let collecting = replay(times, 5, true).unwrap();

        for ((alone, mixed), gc) in read_only
            .reads
            .iter()
            .zip(&spare.reads)
            .zip(&collecting.reads)
        {
            for read in [mixed, gc] {
                assert_eq!((read.arrival, read.lba), (alone.arrival, alone.lba));
                assert_eq!(read.service.operation, alone.service.operation);
                assert_eq!(read.service.die, alone.service.die);
                if alone.service.die == 1 {
                    assert_eq!(read.service, alone.service);
                }
            }
        }
        assert_eq!(read_only.reads.len(), (TICKS * 2) as usize);
        assert_eq!(spare.reads.len(), read_only.reads.len());
        assert_eq!(collecting.reads.len(), read_only.reads.len());
        assert_eq!(read_only.host_programs, 0);
        assert_eq!(spare.host_programs, TICKS as usize);
        assert_eq!(collecting.host_programs, spare.host_programs);
        assert_eq!(
            (spare.gc_copies, spare.gc_programs, spare.gc_erases),
            (0, 0, 0)
        );
        assert!(collecting.gc_erases > 0 && collecting.gc_copies > 0);
        assert_eq!(collecting.gc_copies, collecting.gc_programs);
        let (_, first_collection) = collecting.first_collection.as_ref().unwrap();
        assert!(matches!(
            first_collection
                .iter()
                .map(|service| service.operation)
                .collect::<Vec<_>>()
                .as_slice(),
            [
                PhysicalOp::Read(_),
                PhysicalOp::Program(_),
                PhysicalOp::Erase(_)
            ]
        ));
        assert_eq!(
            collecting.elapsed() - spare.elapsed(),
            (collecting.gc_copies as u64) * u64::from(times.read.get() + times.program.get())
                + (collecting.gc_erases as u64) * u64::from(times.erase.get())
        );
        assert!(read_only.p99() < spare.p99() && spare.p99() < collecting.p99());
    }
}
