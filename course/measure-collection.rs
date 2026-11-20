#[cfg(feature = "indexed-ftl")]
#[path = "../src/free_space.rs"]
mod free_space;
#[path = "../src/ftl.rs"]
mod ftl;
#[path = "../src/nand.rs"]
mod nand;

use std::time::Instant;

use ftl::Ftl;
use nand::{NandGeometry, BLOCK_BYTES};

type Trace = Vec<String>;

fn sample(collect: bool, large: bool) -> u128 {
    let logical = if large { 256 } else { 8 };
    let overwrites = if large { 760 } else { 4 };
    let mut ftl = if large {
        Ftl::with_geometry(logical, NandGeometry::new(128, 8).unwrap()).unwrap()
    } else {
        Ftl::new()
    };
    let mut trace = Trace::with_capacity(48);
    for lba in 0..logical {
        ftl.write(lba, Box::new([lba as u8; BLOCK_BYTES]), &mut trace)
            .unwrap();
    }
    for index in usize::from(!collect)..overwrites {
        ftl.write(0, Box::new([(index + 10) as u8; BLOCK_BYTES]), &mut trace)
            .unwrap();
    }
    let trigger_start = trace.len();
    let data = Box::new([42; BLOCK_BYTES]);
    let begin = Instant::now();
    ftl.write(1, data, &mut trace).unwrap();
    let elapsed = begin.elapsed().as_nanos();

    let expected = if large && collect {
        &[
            "nand   erase block 32, reclaim 8 stale pages",
            "nand   program page 256",
            "ftl    map LBA 1 -> page 256, stale page 1",
        ][..]
    } else if large {
        &[
            "nand   program page 1015",
            "ftl    map LBA 1 -> page 1015, stale page 1",
        ][..]
    } else if collect {
        &[
            "ftl    gc copy LBA 0 from page 11 to page 12",
            "nand   erase block 2, reclaim 3 stale pages",
            "nand   program page 8",
            "ftl    map LBA 1 -> page 8, stale page 1",
        ][..]
    } else {
        &[
            "nand   program page 11",
            "ftl    map LBA 1 -> page 11, stale page 1",
        ][..]
    };
    assert_eq!(&trace[trigger_start..], expected);

    ftl.flush(&mut trace);
    ftl.write(1, Box::new([99; BLOCK_BYTES]), &mut trace)
        .unwrap();
    assert_eq!(*ftl.read(1, &mut trace).unwrap(), [99; BLOCK_BYTES]);
    trace.clear();
    let recovered = Ftl::open(ftl.power_cut(), &mut trace).unwrap();
    assert!(trace.is_empty(), "fixture retains the erase-block reserve");
    for lba in 0..logical {
        let byte = match lba {
            0 => (overwrites - 1 + 10) as u8,
            1 => 42,
            _ => lba as u8,
        };
        assert_eq!(
            *recovered.read(lba, &mut trace).unwrap(),
            [byte; BLOCK_BYTES]
        );
    }
    elapsed
}

fn main() {
    let sweep = std::env::args().nth(1).expect("sweep number");
    let fixture = std::env::args().nth(2).expect("fixture name");
    let large = fixture == "large";
    let indexed = cfg!(feature = "indexed-ftl");
    let implementation = if indexed { "indexed" } else { "scan" };
    for pair in 0..59 {
        for (position, collect) in [pair % 2 == 0, pair % 2 != 0].into_iter().enumerate() {
            let ns = sample(collect, large);
            if pair >= 8 {
                let (page_slots, map_slots, summaries, free_entries) =
                    match (large, indexed, collect) {
                        (false, false, true) => (70, 40, 0, 0),
                        (false, false, false) => (28, 0, 0, 0),
                        (false, true, true) => (0, 8, 8, 2),
                        (true, false, true) => (3329, 33024, 0, 0),
                        (true, false, false) => (2040, 0, 0, 0),
                        (true, true, true) => (0, 256, 256, 1),
                        (_, true, false) => (0, 0, 0, 1),
                    };
                let erase = usize::from(collect);
                let relocation = usize::from(collect && !large);
                println!(
                    "{fixture},{sweep},{implementation},{},{position},{erase},{ns},{page_slots},{map_slots},{summaries},{free_entries},{},{erase}",
                    pair - 8,
                    1 + relocation,
                );
            }
        }
    }
}
