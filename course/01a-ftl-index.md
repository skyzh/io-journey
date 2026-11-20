# Track free space and live pages

Finish [NAND to SSD](01-ftl.md) and run `./course/learner.sh check`.
If you used the original mutable map in the first chapter, your edits
still work on the scan path. Before you enable indexing, use
`self.map.replace(lba, page)` for mapping updates in your FTL. In the
relocation loop, iterate `0..self.map.len()` and read each old location
with `self.map[lba]`. This gives each remap one place to update its counts.
The scan-based FTL counts erased pages before writing and walks the
mapping for every possible erase victim. An ordered set of erased page
addresses supplies the reserve and first free page directly. Counts of
occupied and live pages give each victim's stale-page count.

## Maintain the index

Edit [`course/learner/index.rs`](learner/index.rs). Its predecessor is
your working FTL. The new check compares the index with the physical
pages and mappings after each transition.

```sh
./course/learner.sh index
```

The starter fails on its first programmed page. Make these updates at
the points where the scaffold calls your methods.

1. In `record_program`, remove the successfully programmed page from the
   free set and increment the occupied count for its block. A torn page
   is occupied even when the LBA mapping stays unchanged.
2. In `record_erase`, add every page in the erased block to the free set
   and reset that block's occupied count.
3. In `record_remap`, subtract one live mapping from the previous page's
   block when there was one. Add one to the new page's block. Relocation
   uses the same mapping operation after programming its destination.

Run the check again. It crosses an overwrite, collection, flush, export,
tear, and reopen. The saved images each carry the index for their own
pages and mappings. Compare the saved image and crash image after the
tear. Only the crash image has the newly occupied partial page.

## Measure the same work

Run the comparison on Linux from the checkout. Redirect the CSV to a
path on guest tmpfs if you want to save it.

```sh
./course/measure-collection.sh
```

The script compares the two supplied reference implementations. Use
`./course/learner.sh index` to check your own code. The benchmark checks
the complete reference trace and its exact page locations.

Both complete paths run the same fixture from the reference FTL
measurement. There are eight LBAs and four erase blocks of four pages.
Fill LBAs 0–7, overwrite LBA 0 with 10 through 13, and time the write of
42 to LBA 1. A second arm starts at 11 so the timed request fits without
collection. Within each arm the two implementations receive the same
requests. Both recover all eight acknowledged blocks after a volatile
overwrite. The timed interval includes the write and trace creation.

The second fixture uses 256 LBAs and 128 erase blocks of eight pages.
Fill the LBAs, then overwrite LBA 0 760 times before the collection
arm. The control starts at the second overwrite. Both arms write 42
to LBA 1 at the timing boundary and recover all 256 LBAs. In this
geometry the selected victim contains only stale pages, so collection
can reclaim it without relocating a live page.

The trace checks the physical placement and records programs and
erases. The slot and index counts follow that checked trace and the
loops that visit its exact addresses. They count physical or map slots,
summary reads, and returned free entries. Internal ordered-tree lookups,
point accesses to copy and program data, and allocator work have
separate costs. Compare the timings as well as the reduced scans.

### Recorded comparison

This measured example uses 102 writes per implementation and condition
across two order-balanced sweeps on OrbStack, with Rust 1.93.1 and
identical release build flags.
The source measured was `700f0fac2fe729fc5f46400913767c91e9c0b60f`.

| Geometry and timed write | NAND slots visited during search and selection, scan / indexed | Map slots, scan / indexed | Programs / erases in both | Median time, scan / indexed |
| --- | ---: | ---: | ---: | ---: |
| 16 pages, collect | 70 / 0 | 40 / 8 | 2 / 1 | 292 / 333 ns |
| 16 pages, no collection | 28 / 0 | 0 / 0 | 1 / 0 | 42 / 83 ns |
| 1,024 pages, collect | 3,329 / 0 | 33,024 / 256 | 1 / 1 | 14,584 / 625 ns |
| 1,024 pages, no collection | 2,040 / 0 | 0 / 0 | 1 / 0 | 500 / 125 ns |

The indexed collection reads eight block summaries in the small case
and 256 in the large case. It examines two free entries in the small
case because a live page needs a destination, and one in the large
case. The initial run of the small fixture also showed higher indexed
collection time, with medians of 667 and 708 ns, and tied at 167 ns
without collection. A separate learner run found that the small-geometry
timings tied. Small workloads can tie or get slower with an index.
The collection ranges overlap in both author runs of the small fixture.
In the table's sweep they were 208–417 ns for scanning and 209–417 ns
for indexing. Maintaining the index costs work at every update.
Its reduced scans become useful at the larger geometry in this
fixture. The small workload follows the FTL case at `4d04bed`.
[Fixture source](measure-collection.rs) · [Run script](measure-collection.sh)
