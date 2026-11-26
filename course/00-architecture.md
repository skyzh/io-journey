# Overview: one write through the stack

When an application appends to a file and calls sync, its bytes pass
through several layers before they rest on flash. In this course you build
four of those layers in Rust and run them against a simulated NAND chip:

<ol class="write-path" aria-label="One append through the storage stack">
  <li><strong>Application</strong><span>Write the record bytes at the end of the log, then sync the file</span></li>
  <li><strong>Filesystem</strong><span>Translate the byte offset into a logical block and update the file length</span></li>
  <li><strong>Block queue</strong><span>Send 4 KiB block requests, then flush the earlier writes</span></li>
  <li><strong>Flash translation layer</strong><span>Map each updated block to a fresh page</span></li>
  <li><strong>Simulated NAND</strong><span>Program pages and erase whole blocks</span></li>
</ol>

The application acknowledges the records after the flush completes and
file sync returns.

Each layer talks to the one below it in a smaller vocabulary. The
application knows files and bytes. The filesystem turns them into numbered
4 KiB logical blocks. The queue decides which block request runs next. The
flash translation layer (FTL) decides where on the chip each logical block
lives. The NAND model stores pages and enforces the rules real flash has.
512-byte logical sectors are common, including on drives that emulate
them over 4 KiB physical sectors. The course simplifies the model by
using 4 KiB logical blocks, each sized to fit one simulated NAND page.
The Linux ublk adapter advertises a 4 KiB logical sector. The application
writes bytes at a file offset, and the filesystem sends whole-block
requests to the device.
[Logical and physical sector sizes](https://learn.microsoft.com/en-us/windows-hardware/manufacture/desktop/hard-drives-and-partitions?view=windows-11)

## Follow one append

The [reference demo](START.md#run-the-reference-demo) appends two records
to `/events.log` and syncs the file. The filesystem keeps the file length
in logical block 0 (LBA 0) and the record bytes in LBA 1. Watch where
those two blocks move as the second record is appended.

### 1. Read the block holding the first record

The first record takes 13 bytes (an 8-byte header and the 5-byte payload
`first`). By the time request #4 reads LBA 1, the queue has completed the
first record’s data and length writes. Here is the live mapping then.

<figure class="block-map">
  <figcaption>Mapping when the second append reads LBA 1</figcaption>
  <ul>
    <li><div><strong>LBA 0</strong><span>File length: 13 bytes</span></div><div><strong>NAND page 2</strong><span>Directory block</span></div></li>
    <li><div><strong>LBA 1</strong><span>Record bytes</span></div><div><strong>NAND page 1</strong><span>Holds <code>first</code></span></div></li>
  </ul>
</figure>

### 2. Append at byte offset 13 and write two blocks

The second record takes 14 bytes (the header plus `second`). The
filesystem copies those bytes into the block it read, starting at
offset 13. It queues the updated data block (#5) and the new file
length (#6). The FTL writes both blocks to fresh NAND pages. Pages 1
and 2 become stale as their mappings move.

<figure class="block-map">
  <figcaption>Mapping after writes #5 and #6, saved by flush #7</figcaption>
  <ul>
    <li><div><strong>LBA 0</strong><span>File length: 27 bytes</span></div><div><strong>NAND page 4</strong><span>Replaces page 2</span></div></li>
    <li><div><strong>LBA 1</strong><span>Record bytes</span></div><div><strong>NAND page 3</strong><span>Holds <code>first</code> and <code>second</code></span></div></li>
  </ul>
</figure>

### 3. Sync the file, then recover the saved mapping

The application syncs the file. The queue runs its flush (#7) after
both block writes complete. The FTL saves the two new mappings and
their page contents in the simulated durable image. Once sync returns,
the application acknowledges both records.

The demo then appends `volatile` without syncing again. That append is
readable before the cut. After the cut, the FTL reopens from the saved
image and follows the mappings back to pages 4 and 3. The recovered
file has length 27 and contains only `first` and `second`. The demo
then tries a torn page program and verifies that the same two records
survive.

## What NAND allows

NAND flash has three operations, and the course's model in
[`src/nand.rs`](https://github.com/skyzh/io-journey/blob/main/src/nand.rs)
enforces simplified versions of these limits.

- **Read** a page.
- **Program** an erased page. Erased flash reads as `0xFF` bytes, and
  programming changes one bits to zero. Some NAND devices permit a limited
  number of partial-page programs before the next erase.
  [Macronix datasheet, Table 18](https://www.macronix.com/Lists/Datasheet/Attachments/9075/MX35LF1GE4AB%2C%203V%2C%201Gb%2C%20v2.0.pdf#page=44)
  The model allows one program per page between erases. For a valid page
  index, `Nand::read` returns `None` for an erased page.
- **Erase** a whole erase block at once. Here a block has 4 pages; real
  chips have far more. There is no way to erase a single page.

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

Chapter 1 has you write the mapping update and the copy step.

## What "durable" means here

A real SSD typically works from a copy of its mapping held in RAM. RAM is
lost at power-off, so the drive must also write enough information to the flash
itself to rebuild the mapping after a sudden power loss. SPDK provides a
software FTL over block devices. After a power failure it rebuilds the
mapping from reverse maps for closed bands on the base device and chunks
on the cache device. Sequence IDs identify the newest entry for each LBA.
Open bands use a separate metadata region. Open chunks recover from cache
metadata or a log.
[SPDK recovery design](https://spdk.io/doc/ftl.html#ftl_dirty_shutdown)

This course's model takes a shortcut. A flush saves a copy of the NAND
pages and the mapping together, in RAM, as one step. A simulated power cut
keeps only that copy, plus any partial bytes a torn program left on its
page. This is enough to show what a flush promises: which writes survive
a cut, and why a half-finished program must not replace a complete one.
It doesn't show how a drive stores its own mapping on flash. That would be
a separate exercise.

## What you build

| Chapter | Layer | You add |
| --- | --- | --- |
| [1. NAND to SSD](01-ftl.md) | FTL | Publish each write's new page, and move live pages before an erase |
| [2. Blocks in flight](02-queue.md) | Block queue | Bounded requests, flush ordering, and Linux tag dispatch |
| [3. Blocks to files](03-filesystem.md) | Filesystem | A journal that makes file and directory changes recoverable |
| [4. Efficient application I/O](04-append.md) | Application | Record recovery and an append engine that acknowledges after sync |

Chapter 1 starts on a complete NAND model. Each later chapter runs on
the code you wrote in the earlier ones.

Next, [set up your environment and run the demo](START.md).
