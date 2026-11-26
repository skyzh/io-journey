# 1. NAND to SSD

By the end of this chapter, the simulated device behaves like a small SSD.
You can overwrite any logical block, and the FTL writes the new data to a
fresh NAND page and records where it went. When erased pages run low, the
FTL frees space without losing data.

## What NAND allows

NAND flash has three operations, and the course's model in
[`src/nand.rs`](https://github.com/skyzh/io-journey/blob/main/src/nand.rs)
enforces simplified versions of these limits.

- **Read** a page.
- **Program** an erased page. Erased flash reads as `0xFF` bytes, and
  programming can only change bits from 1 to 0. Some NAND devices permit a limited
  number of partial-page programs before the next erase.
  [Macronix datasheet, Table 18](https://www.macronix.com/Lists/Datasheet/Attachments/9075/MX35LF1GE4AB%2C%203V%2C%201Gb%2C%20v2.0.pdf#page=44)
  The model allows one program per page between erases. For a valid page
  index, `Nand::read` returns `None` for an erased page.
- **Erase** a whole erase block at once. Here a block has 4 pages; real
  chips have far more. There is no way to erase a single page.

## The device

The device has 8 logical blocks (LBAs 0–7) of 4 KiB each, stored on 16
physical NAND pages. The pages form 4 erase blocks of 4 pages:

| Erase block | NAND pages |
| --- | --- |
| 0 | 0–3 |
| 1 | 4–7 |
| 2 | 8–11 |
| 3 | 12–15 |

There are twice as many pages as LBAs, so an overwrite always has
somewhere new to go, and collection always has room to work.

512-byte logical sectors are common, including on drives that emulate
them over 4 KiB physical sectors. The course simplifies the model by
using 4 KiB logical blocks, each sized to fit one simulated NAND page.
The Linux ublk adapter advertises a 4 KiB logical sector.
[Logical and physical sector sizes](https://learn.microsoft.com/en-us/windows-hardware/manufacture/desktop/hard-drives-and-partitions?view=windows-11)

## Implement the NAND operations

Start in [your course checkout](START.md). The supplied code allocates
the page slots and computes which pages belong to each erase block.
In [`course/learner/nand.rs`](https://github.com/skyzh/io-journey/blob/main/course/learner/nand.rs),
you implement the operations on those slots.

```sh
cargo xtask learner nand
```

The starter fails with `implement NAND page programming`. Work through
the operations and rerun the same check after each edit.

1. **Program a page.** Use the physical page number to look up a mutable
   slot. Return an `InvalidInput` error if the page is outside the
   device. Return an error if the slot is already occupied and leave
   its bytes intact. For an erased slot, store
   `Some(super::storage::keep(data))`.
2. **Read a page.** Return `None` for an erased slot or a page outside
   the device. For a programmed slot, use `super::storage::borrow`
   to return its block without copying it.
3. **Erase a block.** The caller passes a mutable slice for that whole
   block. Set each slot to `None`. The caller selects the slice, so your
   method touches only the pages in that block.

The model uses `None` to represent an erased page. A physical erased page
reads as all `0xFF` bytes. The tests check that reprogramming an occupied
page fails, that erasing one block preserves the adjacent block, and that
you can program a page again after erase.

When both NAND tests pass, you have the device operations the FTL needs.
The FTL keeps track of which physical page holds each logical block.

## Connect the FTL to NAND

The FTL gives the host a stable address for each 4 KiB block while
moving its data underneath. It does this in three parts.

- **Out-of-place writes.** Every write, including an overwrite, programs
  a fresh erased page.
- **A mapping.** A table records which page currently holds each LBA.
  Reads follow the table. An overwrite changes one entry and leaves the
  old page stale.
- **Garbage collection.** Stale pages are wasted space, and they can only
  be reclaimed by erasing their whole block. When erased pages run low,
  the FTL picks a block with stale pages and copies any live pages in it
  somewhere else. Then it updates their mappings and erases the block.
  In this geometry ordinary writes start collection at four or fewer
  erased pages, one erase block's worth. A torn program can use one of
  those pages before collection runs. If fewer than four erased pages
  remain, reopening the device collects to restore the reserve.

Most of the FTL is supplied. You write two methods in
[`course/learner/ftl.rs`](https://github.com/skyzh/io-journey/blob/main/course/learner/ftl.rs):

| Supplied | Yours |
| --- | --- |
| The NAND geometry, page storage, and erased-page iterator in [`src/nand.rs`](https://github.com/skyzh/io-journey/blob/main/src/nand.rs) | The read, program, and erase operations you just implemented |
| `Ftl::write` in [`src/ftl.rs`](https://github.com/skyzh/io-journey/blob/main/src/ftl.rs): check the LBA, pick the first erased page, and program it | `publish_mapping`: point the LBA at that page and return the page it used before |
| Start collection when 4 or fewer erased pages remain, choose the erase block with the most stale pages, and erase it afterwards | `relocate_live_pages`: copy each live page out of that block and update its mapping |
| Reads, flush, simulated power cuts, the torn-program fault, and reopen | |

The mapping is `self.map`, a table with one entry per LBA. Each entry is
`Some(page)` or `None` for a block that was never written.

## Run the check

With both NAND tests passing in this checkout, run:

```sh
cargo xtask learner map
```

The mapping check,
`successful_programs_publish_the_new_mapping`, stops with
`map the first programmed page`. The write programmed a page, but the
mapping still says the LBA has no data. The test is in `src/ftl.rs`.
Make your edits in `course/learner/ftl.rs`.

## Step 1: publish the new page

Here is what a correct FTL prints when the 8 LBAs are written once and
LBA 0 is then overwritten:

```text
nand   program page 0
ftl    map LBA 0 -> page 0
...
nand   program page 7
ftl    map LBA 7 -> page 7
nand   program page 8
ftl    map LBA 0 -> page 8, stale page 0
```

The overwrite doesn't touch page 0. `Ftl::write` programs the new bytes
into page 8, the first erased page, and then calls your method.
`publish_mapping(lba, page)` must make the LBA point to `page` and return
the LBA's previous page: `Some(0)` here, or `None` on a first write.
`Ftl::write` uses that return value to print the `stale page` part. By
the time your method runs, the LBA has been checked and the page has
been programmed successfully.

Rename the `_page` parameter to `page` and use
`self.map.replace(lba, page)`. It updates the entry and returns the old
one.

Rerun `cargo xtask learner map`. When it passes, run
`cargo xtask learner collect`. The collection fixture writes enough
to run out of spare pages and reaches `implement live-page relocation`.

## Step 2: move live pages before an erase

Keep overwriting LBA 0 and erased pages run out. Continue the trace above
until LBA 0 has been written to pages 8, 9, 10 and 11. The chip now looks
like this:

| Erase block | Stale pages | Live pages | Erased pages |
| --- | --- | --- | --- |
| 0 | 0 | 1–3 hold LBAs 1–3 | |
| 1 | | 4–7 hold LBAs 4–7 | |
| 2 | 8–10 | 11 holds LBA 0 | |
| 3 | | | 12–15 |

Only 4 erased pages are left, so the next write, LBA 1, first starts
collection. Erase block 2 has the most stale pages, 3, so the supplied
code chooses it. Page 11 is still live: LBA 0 points to it. Erasing the
block now would lose LBA 0. Your method has to copy it out first:

```text
ftl    gc copy LBA 0 from page 11 to page 12
nand   erase block 2, reclaim 3 stale pages
nand   program page 8
ftl    map LBA 1 -> page 8, stale page 1
```

LBA 0 moves to page 12, block 2 is erased, and the write of LBA 1 lands
on the newly erased page 8. The `gc copy` line comes from the reference
FTL. Your version prints it only if you push a line to `_trace`; that
parameter is optional to use.

`relocate_live_pages(victim_pages, _trace)` gets the victim's page range,
`8..12` here. For each LBA whose mapping points into that range:

1. Choose an erased destination outside the victim, using
   `self.nand.erased_pages()`.
2. Read the source with `self.nand.read(source)`. It returns
   `Option<&Block>`. For a valid page index, `None` means the page is erased.
   A source from the live map is a valid, programmed page, so unwrap it
   with `.expect("mapped page")`. Clone the block to give `program` an owned
   `Block`.
3. Program the destination with `self.nand.program(destination, data)`.
   Return its error with `?` if it fails.
4. Only after the program succeeds, record the move with
   `self.map.replace(lba, destination)`.

Visit the LBAs with `for lba in 0..self.map.len()` and read each entry with
`self.map[lba]`. Stale pages need no work, because no LBA points to them
and the erase reclaims them with the rest of the block. Victim selection
has already checked that enough erased pages exist outside the victim.
The caller erases the block only if your method returns `Ok`.

Rerun `cargo xtask learner collect`. The full FTL suite at the end of
this chapter also checks that:

- every LBA reads back its expected bytes after the image is reopened;
- a torn program, power lost partway through writing a page, leaves those
  exact partial bytes on the page while the LBA still reads its previous
  complete value;
- a torn program is refused when no page is erased in both the live and
  the flushed image;
- repeated torn programs still leave room for another durable write.

The supplied code handles the torn programs. The tests check that your
mapping and relocation keep them safe.

## What a flush saves

An SSD keeps all or part of its mapping in RAM while it runs. RAM is lost
at power-off, so the drive also writes enough metadata to persistent media
to rebuild the mapping after a sudden power loss. SPDK's software FTL, for
example, [pages mapping entries in and out of a RAM cache](https://spdk.io/doc/ftl.html#ftl_l2p)
and [rebuilds the map from persistent metadata after a dirty shutdown](https://spdk.io/doc/ftl.html#ftl_dirty_shutdown).

This course's model takes a shortcut. A flush saves a copy of the NAND
pages and the mapping together, in RAM, as one step. A simulated power cut
keeps only that copy, plus any partial bytes a torn program left on its
page. This is enough to show what a flush promises: which writes survive
a cut, and why a half-finished program must not replace a complete one.
It doesn't show how a drive stores its own mapping on flash. That would be
a separate exercise.

### What survives a torn program?

Write a block filled with `0x12` to LBA 1 on an empty device. The FTL
programs page 0 and maps LBA 1 to it. A successful flush copies the NAND
image and the logical-to-physical map together into a durable checkpoint.
The simulator holds that checkpoint in RAM to model what survives a cut.

Now overwrite LBA 1 with a block filled with `0x34` and cut power after
the first 19 bytes are programmed. The simulator records those bytes in
a page that was erased in both the live image and the checkpoint. The
rest of the page still contains erased bytes. The failed program leaves
the checkpoint's mapping at LBA 1 → page 0.

Reopen from the checkpoint after the cut. The simulator restores its NAND
image, including the partial page, and loads the saved map directly.
A read of LBA 1 still returns the full `0x12` block from page 0.

Run the existing check against the supplied implementation from your Linux
checkout:

```sh
cargo xtask test reference a_torn_program_persists_the_exact_prefix_without_publishing_a_mapping
```

The check inspects the exact bytes in the partial page and reads LBA 1
after reopening. The focused suite below runs the same check with your FTL.

## Check your FTL

Run the focused FTL suite from the checkout after both exercises.

```sh
cargo xtask learner check
```

## Why requests wait

The timing model gives each NAND die one operation at a time, and makes a
program or erase take far longer than a read. Run it to see what that does
to reads:

```sh
cargo xtask nand-latency
```

It replays the same 48 reads three times on a two-die model with
illustrative service times: read 80 µs, program 400 µs, erase 1,600 µs.
Look at the `read p99 us` column. With reads alone it is 80 µs. Add
overwrites and it rises to 6,895 µs, because reads on the busy die wait
behind programs. Let garbage collection run as well and it reaches
24,175 µs, because reads also wait behind copies and erases. In the
"First collection" lines, the read on die 1 runs as soon as it arrives,
while the read on die 0 waits for the whole collection.

The numbers come from a fixed simulated trace, not from a real SSD. In
this model, work queues up per die, and the order in which requests are
issued decides who waits. Ordering requests is the next layer.

When the eight FTL tests pass, continue to [Blocks in flight](02-queue.md).
You’ll use this FTL as the device behind your request queue. In the
[final chapter](04-append.md), you’ll run the completed stack end to end
and check that it recovers the records the application acknowledged.
