# Overview: one write through the stack

When an application appends to a file and calls sync, its bytes pass
through several layers before they rest on flash. In this course you build
four of those layers in Rust and run them against a simulated NAND chip:

```text
application    append "second" to /events.log, then sync
    │          bytes at a file offset
filesystem     which 4 KiB logical blocks change, and the new file length
    │          block requests: read LBA 1, write LBA 1, write LBA 0, flush
block queue    at most two requests in flight; a flush waits for earlier writes
    │          one 4 KiB write to one logical block address (LBA)
FTL            choose an erased page, program it, point the LBA at it
    │          program page 3, erase block 2
NAND           16 pages of 4 KiB, grouped into 4 erase blocks of 4 pages
```

Each layer talks to the one below it in a smaller vocabulary. The
application knows files and bytes. The filesystem turns them into numbered
4 KiB logical blocks. The queue decides which block request runs next. The
flash translation layer (FTL) decides where on the chip each logical block
lives. The NAND model stores pages and enforces the rules real flash has.

## Follow one append

Run the demo from the [setup page](START.md) and find the second record.
These are selected lines from `cargo xtask demo`, in the order they appear:

```text
app    append 6 payload bytes
fs     append 14 bytes at offset 13
queue  submit #4 read LBA 1 (2/2)
ftl    read LBA 1 from page 1
queue  submit #5 write LBA 1 (1/2)
fs     queue directory length 27
queue  submit #6 write LBA 0 (2/2)
fs     sync /events.log
nand   program page 3
ftl    map LBA 1 -> page 3, stale page 1
queue  submit #7 flush (2/2)
nand   program page 4
ftl    map LBA 0 -> page 4, stale page 2
ftl    flush published NAND pages and mapping in RAM
app    acknowledged 2 records after flush
```

Reading from the top:

- **Application.** The payload `second` is 6 bytes. The record adds an
  8-byte header holding its length and a CRC-32, so 14 bytes go to the file.
- **Filesystem.** The file already holds the 13-byte first record, so the
  new bytes land at offset 13. That is inside the file's first data block,
  LBA 1, which already holds other bytes. The filesystem reads LBA 1,
  changes 14 bytes, and writes the whole block back. Then it writes the
  new length, 27, into its directory block, LBA 0. Sync asks for a flush.
- **Queue.** Each read, write and flush becomes a numbered request. The
  `(2/2)` means both queue slots are now full. The flush, request #7,
  runs only after the writes submitted before it.
- **FTL.** LBA 1 was stored on page 1. The FTL does not touch page 1. It
  programs the new block into an erased page, page 3, and moves LBA 1's
  pointer there. Page 1 still holds the old bytes, but nothing points to
  it any more: it is *stale*. LBA 0 moves from page 2 to page 4 the same way.
- **Flush.** Once the flush completes, the application treats both
  records as durable and acknowledges them.

The FTL's behavior, writing somewhere new instead of overwriting, comes
straight from how NAND works.

## What NAND allows

NAND flash has three operations, and the course's model in
[`src/nand.rs`](https://github.com/skyzh/io-journey/blob/main/src/nand.rs) enforces the same limits:

- **Read** a page.
- **Program** a page, but only if it is erased. Erased flash reads as all
  one bits, `0xFF` bytes, and programming can only clear bits. A page
  that holds data can't be rewritten in place. In the model,
  `Nand::program` returns an error for a page that isn't erased.
- **Erase** a whole erase block at once. Here a block has 4 pages; real
  chips have far more. There is no way to erase a single page.

So a disk that lets you overwrite any 4 KiB block at any time has to be
built on top of a chip that can't overwrite anything. That is the FTL's
job, and it does it in three parts:

- **Out-of-place writes.** Every write, including an overwrite, programs
  a fresh erased page.
- **A mapping.** A table records which page currently holds each LBA.
  Reads follow the table. An overwrite changes one entry and leaves the
  old page stale.
- **Garbage collection.** Stale pages are wasted space, and they can only
  be reclaimed by erasing their whole block. When erased pages run low,
  the FTL picks a block with stale pages and copies any live pages in it
  somewhere else. Then it updates their mappings and erases the block.
  This FTL keeps at least one erase block's worth of pages (4) erased,
  so those copies always have room.

Chapter 1 has you write the mapping update and the copy step.

## What "durable" means here

A real SSD typically works from a copy of its mapping held in RAM. RAM is
lost at power-off, so the drive must also write enough information to the flash
itself to rebuild the mapping after a sudden power loss. One documented
design is the [SPDK FTL](https://spdk.io/doc/ftl.html) (see "Dirty
shutdown recovery"). Each band, a large region of flash, keeps a
physical-to-logical map: which LBA each page holds, with a sequence number
for its age. After a crash the FTL rebuilds the mapping from those maps,
and the newest entry for each LBA wins.

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
the code you wrote in the earlier ones. Start with [NAND to SSD](01-ftl.md).
