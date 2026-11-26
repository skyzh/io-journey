# 1. NAND to SSD

By the end of this chapter, the simulated device behaves like a small SSD.
You can overwrite any logical block, and the FTL writes the new data to a
fresh NAND page and records where it went. When erased pages run low, the
FTL frees space without losing data. If you haven't read the
[overview](00-architecture.md), its "What NAND allows" section explains why
an SSD works this way.

## The device

The device has 8 logical blocks (LBAs 0–7) of 4 KiB each, stored on 16
physical NAND pages. The pages form 4 erase blocks of 4 pages:

```text
erase block 0   pages  0  1  2  3
erase block 1   pages  4  5  6  7
erase block 2   pages  8  9 10 11
erase block 3   pages 12 13 14 15
```

There are twice as many pages as LBAs, so an overwrite always has
somewhere new to go, and collection always has room to work.

Most of the FTL is supplied. You write two methods in
[`course/learner/ftl.rs`](https://github.com/skyzh/io-journey/blob/main/course/learner/ftl.rs):

| Supplied | Yours |
| --- | --- |
| The NAND model in [`src/nand.rs`](https://github.com/skyzh/io-journey/blob/main/src/nand.rs): `read`, `program`, `erase`, and `erased_pages` | |
| `Ftl::write` in [`src/ftl.rs`](https://github.com/skyzh/io-journey/blob/main/src/ftl.rs): check the LBA, pick the first erased page, and program it | `publish_mapping`: point the LBA at that page and return the page it used before |
| Start collection when 4 or fewer erased pages remain, choose the erase block with the most stale pages, and erase it afterwards | `relocate_live_pages`: copy each live page out of that block and update its mapping |
| Reads, flush, simulated power cuts, the torn-program fault, and reopen | |

The mapping is `self.map`, a table with one entry per LBA. Each entry is
`Some(page)` or `None` for a block that was never written.

## Run the check

Work in the checkout from [environment setup](START.md) and run:

```sh
cargo xtask test
```

The supplied NAND check passes. The first FTL check,
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

Run `cargo xtask test` again. The mapping check passes and the live-page
check reaches `implement live-page relocation`. Its fixture wrote enough
to need collection.

## Step 2: move live pages before an erase

Keep overwriting LBA 0 and erased pages run out. Continue the trace above
until LBA 0 has been written to pages 8, 9, 10 and 11. The chip now looks
like this:

```text
erase block 0   page 0 stale    pages 1–3 hold LBAs 1–3
erase block 1   pages 4–7 hold LBAs 4–7
erase block 2   pages 8–10 stale    page 11 holds LBA 0
erase block 3   pages 12–15 erased
```

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

Run `cargo xtask test`. The FTL suite should report 8 passing tests.
The next failing check is now in the queue chapter. Besides the two
traces above, the FTL suite checks that:

- every LBA reads back its expected bytes after the image is reopened;
- a torn program, power lost partway through writing a page, leaves those
  exact partial bytes on the page while the LBA still reads its previous
  complete value;
- a torn program is refused when no page is erased in both the live and
  the flushed image;
- repeated torn programs still leave room for another durable write.

The supplied code handles the torn programs. The tests check that your
mapping and relocation keep them safe.

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
after reopening. After you finish both exercises, `cargo xtask test`
runs the same check with your FTL.

## Use your FTL from ext4

After the FTL checks pass, run the ext4 lab from this checkout. It uses
a disposable guest ublk device. Check the [ublk prerequisites](https://github.com/skyzh/io-journey/blob/main/linux/ublk/README.md)
for the kernel device, ext4 tools, libclang, and temporary Cargo dependencies.
Use the [root guest shell](START.md#mounted-labs) from the checkout.
The ublk service stores its runtime metadata in `/run`, which must be on tmpfs.

```sh
cargo xtask ext4 learner
```

The `learner` argument builds the adapter with `course/learner/ftl.rs`.
A missing or unbuildable implementation stops the build before a device is
allocated.

Look for the learner mapping trace, then the saved checkpoint from the app's
sync. The script reads the unsynced third record, unmounts, and reopens the
saved image. The read-only remount must recover `first` and `second` with
their checksums. The small ext4 workload checks placement and recovery.
The offline collection fixture forces live-page relocation separately.

## Next

[Blocks in flight](02-queue.md) puts a bounded request queue on top of
your FTL.
