# Share immutable checkpoint pages

Finish [NAND to SSD](01-ftl.md) and run `cargo xtask learner check`.
The starting FTL copies the NAND pages and map on every flush and
copies them again when you separately export a checkpoint. Even a
clean flush copies every programmed page, including stale ones.

## Share the bytes and keep separate images

Open [`course/learner/pages.rs`](learner/pages.rs) and run the new check.

```sh
cargo xtask learner sharing
```

The starter preserves the bytes but fails the check that clean exports
share the same stored page. Keep the existing owned `Block` at the I/O
boundary. Inside NAND, make `StoredPage` a shared immutable owner
using Rust's `Arc`. Move the existing `Block` into that owner in
`keep`. The supplied `borrow` returns its bytes through a shared
reference. Cloning an image clones the page handles and the map. It
keeps its own slots, so programming a new page or clearing an erase
block changes the live image while the saved image keeps its bytes.

Run the check again. It retains an earlier export through programming
and collection, then keeps a fresh export across a 19-byte tear. The
old export still has the page collection erased. The fresh export
leaves the torn page erased, and the internal crash image contains the
partial program with the old mapping. Reopen recovers the full committed
blocks. The same check runs journal recovery and replay on the learner's
shared pages.

## Measure where the work goes

```sh
cargo xtask ftl-snapshots timing
cargo xtask ftl-snapshots heap
```

The two complete implementations use identical geometry, request bytes,
and save lifetimes. Fill 256 LBAs on 128 blocks of eight pages, then run
12 barriers. The clean barriers have no new writes. Before each dirty
barrier, replace one LBA. Hold the first and last exports across the
same tear. Each run checks both saved images and every committed block.
The benchmarks compare the supplied implementations with the free-space
index disabled in both builds. Use
`cargo xtask learner sharing` to check your edited implementation.

The timing CSV has one row per barrier. It measures flush and export
separately and includes a write-to-flush interval for the input at that
barrier. Summing those intervals covers the same fill and overwrites
and leaves export time separate. The heap run counts requested
4 KiB page allocations at each barrier and the retained heap on return.
The slot vector and mapping still allocate per image. Shared pages add
an owner allocation when they are first programmed and can stay
retained while an earlier image holds them.

### Recorded comparison

The guest ran two sweeps per implementation with 21 repetitions per sweep.
These medians use identical Rust 1.93.1 release flags.

| Work | Full copy | Shared pages |
| --- | ---: | ---: |
| Clean flush | 25.96 µs | 1.04 µs |
| Clean export | 22.54 µs | 0.63 µs |
| Dirty write and flush | 25.35 µs | 1.46 µs |
| Write-to-flush total over all 12 barriers | 372.23 µs | 81.71 µs |
| New payload allocated per flush or export | 1,048,576–1,069,056 B | 0 B |
| First export's added retained heap | 1,060,864 B | 12,288 B |
| Last export's added retained heap | 1,081,344 B | 12,288 B |
| Last clean flush peak heap in the probe | 4,307,109 B | 1,126,685 B |

The 12 full-copy flushes copied 12,705,792 bytes, and the 12 exports
copied the same amount again. The shared version still clones 8,192 bytes
of physical slots and 4,096 bytes of logical mapping per image. The first
256 programs add 6,144 bytes of shared-owner storage in this build. An
owner stays live as long as a saved image holds it. Allocation counts
refer to requested Rust heap bytes in the probe. An earlier sweep measured total write-to-flush
times of 421.48 µs and 97.44 µs, with identical heap counts. Timings refer to
this simulated workload in OrbStack. The workload follows the deep-copy FTL
case at `34674e0`. [Fixture source](measure-snapshots.rs) · [Run script](measure-snapshots.sh)
