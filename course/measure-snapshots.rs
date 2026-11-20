#[path = "../src/ftl.rs"]
mod ftl;
#[path = "../src/nand.rs"]
mod nand;

use ftl::{Ftl, TornProgram};
use nand::{NandGeometry, BLOCK_BYTES, ERASED_BYTE};
use std::time::Instant;

type Trace = Vec<String>;
const SHARED: bool = cfg!(feature = "shared-pages");

fn implementation() -> &'static str {
    if SHARED {
        "shared"
    } else {
        "copy"
    }
}

#[cfg(allocator_probe)]
mod memory {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    #[derive(Clone, Copy, Default)]
    struct Stats {
        live: usize,
        peak: usize,
        page_allocs: usize,
        page_frees: usize,
    }

    thread_local! {
        static STATS: Cell<Stats> = const { Cell::new(Stats {
            live: 0, peak: 0, page_allocs: 0, page_frees: 0,
        }) };
    }

    fn allocated(layout: Layout) {
        let _ = STATS.try_with(|cell| {
            let mut s = cell.get();
            s.live += layout.size();
            s.peak = s.peak.max(s.live);
            s.page_allocs +=
                usize::from(layout.size() == super::BLOCK_BYTES && layout.align() == 1);
            cell.set(s);
        });
    }

    fn freed(layout: Layout) {
        let _ = STATS.try_with(|cell| {
            let mut s = cell.get();
            s.live -= layout.size();
            s.page_frees += usize::from(layout.size() == super::BLOCK_BYTES && layout.align() == 1);
            cell.set(s);
        });
    }

    struct Probe;

    #[global_allocator]
    static ALLOCATOR: Probe = Probe;

    unsafe impl GlobalAlloc for Probe {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let ptr = System.alloc(layout);
            if !ptr.is_null() {
                allocated(layout);
            }
            ptr
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            System.dealloc(ptr, layout);
            freed(layout);
        }

        unsafe fn realloc(&self, ptr: *mut u8, old: Layout, new_size: usize) -> *mut u8 {
            let next = System.realloc(ptr, old, new_size);
            if !next.is_null() {
                freed(old);
                allocated(Layout::from_size_align(new_size, old.align()).unwrap());
            }
            next
        }
    }

    pub fn begin() -> usize {
        STATS.with(|cell| {
            let live = cell.get().live;
            cell.set(Stats {
                live,
                peak: live,
                ..Stats::default()
            });
            live
        })
    }

    pub fn print(barrier: usize, stage: &str, before: usize, programmed: usize) {
        let s = STATS.with(Cell::get);
        let expected = if super::SHARED { 0 } else { programmed };
        assert_eq!(s.page_allocs, expected, "page clone allocation count");
        println!(
            "{},{barrier},{stage},{before},{},{},{},{},{}",
            super::implementation(),
            s.peak,
            s.live,
            s.page_allocs,
            s.page_frees,
            s.page_allocs * super::BLOCK_BYTES
        );
    }
}

fn verify_lbas(ftl: &Ftl, expected: &[u8], trace: &mut Trace) {
    for (lba, &byte) in expected.iter().enumerate() {
        trace.clear();
        let block = ftl.read(lba, trace).unwrap();
        assert!(
            block.iter().all(|&read_byte| read_byte == byte),
            "LBA {lba}"
        );
    }
}

const PAGES: usize = 1024;
const LBAS: usize = 256;
const BARRIERS: usize = 12;

fn snapshot_sample(sweep: &str, rep: usize, print: bool) {
    let geometry = NandGeometry::new(128, 8).unwrap();
    let mut ftl = Ftl::with_geometry(LBAS, geometry).unwrap();
    let mut trace = Trace::with_capacity(16);
    let mut expected: Vec<u8> = (0..LBAS).map(|lba| lba as u8).collect();
    let before_fill = Instant::now();
    for (lba, &byte) in expected.iter().enumerate() {
        trace.clear();
        ftl.write(lba, Box::new([byte; BLOCK_BYTES]), &mut trace)
            .unwrap();
        assert_eq!(trace.len(), 2);
    }

    let mut first_saved = None;
    let mut latest_saved = None;
    let mut overwritten = 0;
    for index in 0..BARRIERS {
        let before_write = if index == 0 {
            before_fill
        } else {
            Instant::now()
        };
        let stage = if index == 0 {
            "first"
        } else if index % 2 == 0 {
            let lba = index / 2;
            expected[lba] = 0xa0 + lba as u8;
            trace.clear();
            ftl.write(lba, Box::new([expected[lba]; BLOCK_BYTES]), &mut trace)
                .unwrap();
            assert_eq!(trace.len(), 2, "no collection during flush fixture");
            overwritten += 1;
            "dirty"
        } else {
            "clean"
        };
        let barrier = index + 1;
        let programmed = LBAS + overwritten;
        trace.clear();

        #[cfg(allocator_probe)]
        let before = memory::begin();
        let begin = Instant::now();
        ftl.flush(&mut trace);
        let flush_ns = begin.elapsed().as_nanos();
        let write_flush_ns = before_write.elapsed().as_nanos();
        #[cfg(allocator_probe)]
        memory::print(barrier, "flush", before, programmed);
        assert_eq!(
            trace,
            ["ftl    flush published NAND pages and mapping in RAM"]
        );

        #[cfg(allocator_probe)]
        let before = memory::begin();
        let begin = Instant::now();
        let saved = ftl.checkpoint();
        let export_ns = begin.elapsed().as_nanos();
        #[cfg(allocator_probe)]
        memory::print(barrier, "export", before, programmed);

        if print {
            println!(
                "{sweep},{},{rep},{barrier},{stage},{programmed},{},{flush_ns},{export_ns},{write_flush_ns}",
                implementation(), if SHARED { 0 } else { programmed * BLOCK_BYTES }
            );
        }
        assert_eq!(
            (0..PAGES)
                .filter(|&page| saved.physical_page(page).is_some())
                .count(),
            programmed
        );
        if index == 0 {
            first_saved = Some(saved);
        } else if barrier == BARRIERS {
            latest_saved = Some(saved);
        }
    }

    let first_saved = first_saved.unwrap();
    let latest_saved = latest_saved.unwrap();
    trace.clear();
    ftl.arm_torn_write(1, 19).unwrap();
    let error = ftl
        .write(1, Box::new([0x73; BLOCK_BYTES]), &mut trace)
        .unwrap_err();
    let tear = error
        .get_ref()
        .unwrap()
        .downcast_ref::<TornProgram>()
        .unwrap();
    assert_eq!(
        (tear.lba, tear.page, tear.bytes),
        (1, LBAS + overwritten, 19)
    );
    assert!(first_saved.physical_page(tear.page).is_none());
    assert!(latest_saved.physical_page(tear.page).is_none());
    let cut = ftl.power_cut();
    let cut_page = cut.physical_page(tear.page).unwrap();
    assert_eq!(cut_page[..19], [0x73; 19]);
    assert!(cut_page[19..].iter().all(|&byte| byte == ERASED_BYTE));

    for lba in 0..LBAS {
        assert_eq!(first_saved.mapped_page(lba), Some(lba));
        assert_eq!(
            *first_saved.physical_page(lba).unwrap().as_ref(),
            [lba as u8; BLOCK_BYTES]
        );
        let latest_map = if (1..=overwritten).contains(&lba) {
            LBAS + lba - 1
        } else {
            lba
        };
        assert_eq!(latest_saved.mapped_page(lba), Some(latest_map));
        assert_eq!(cut.mapped_page(lba), Some(latest_map));
    }
    assert!((LBAS..PAGES).all(|page| first_saved.physical_page(page).is_none()));
    for page in 0..PAGES {
        if page != tear.page {
            assert_eq!(
                latest_saved.physical_page(page),
                cut.physical_page(page),
                "page {page}"
            );
        }
    }

    trace.clear();
    let reopened = Ftl::open(cut, &mut trace).unwrap();
    assert!(trace.is_empty());
    verify_lbas(&reopened, &expected, &mut trace);
    if print {
        eprintln!(
            "rep={rep} first_saved_programmed={LBAS} last_saved_programmed={} tear_page={} tear_prefix={} recovered_lbas={LBAS}",
            LBAS + overwritten, tear.page, tear.bytes
        );
    }
}

fn main() {
    let sweep = std::env::args().nth(1).expect("sweep number");
    #[cfg(allocator_probe)]
    {
        snapshot_sample(&sweep, 0, false);
        eprintln!(
            "{} heap capture preserved both saved images and the recovered bytes",
            implementation()
        );
    }
    #[cfg(not(allocator_probe))]
    {
        for warmup in 0..3 {
            snapshot_sample(&sweep, warmup, false);
        }
        for rep in 0..21 {
            snapshot_sample(&sweep, rep, true);
        }
    }
}
