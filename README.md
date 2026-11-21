# I/O Journey

Follow an append from a Rust application through a filesystem, a bounded
block queue, and a flash translation layer (FTL) to simulated NAND. The demo
flushes two records and checks their recovery after two simulated power cuts.

[Start here](course/START.md) for the course setup, then follow the
[chapter index](course/README.md).

## Run the demo

From the Linux checkout, complete the [tmpfs setup](course/START.md#set-up-linux),
then run the demo. It has no third-party dependencies.

```sh
cargo xtask demo
```

Run `cargo xtask test` for the offline tests. The demo and tests were
verified on OrbStack Ubuntu 26.04.1 arm64 with Rust 1.93.1.

Look for these lines in the trace.

```text
queue  backpressure at depth 2. Completing the oldest request
app    acknowledged 2 records after flush
app    read before simulated cut: ["first", "second", "volatile"]
app    read after volatile cut: ["first", "second"]
nand   torn NAND program at LBA 1, page 5, after 37 of 4096 bytes
demo   cut left 37 programmed bytes on page 5. LBA 1 recovers from page 3
app    read after torn program: ["first", "second"]
demo   verified the acknowledged prefix after both cuts
```

Continue with the [FTL chapter](course/01-ftl.md) and run your implementation
under ext4. The [independent checkpoints](course/README.md#independent-checkpoints)
and the [reference ext4 lab](linux/ublk/README.md) are also available.
The optional [NAND timing lab](course/01c-nand-latency.md) uses virtual time
to show why programs and garbage collection can delay otherwise identical reads.

## Follow a record

- The application writes a length, a CRC-32, and the payload. Recovery reads
  and checks every complete record in the file.
- The filesystem has one root entry, `/events.log`. Logical block 0 stores
  its length. Seven data blocks store the contents. An append queues data
  writes before the length update.
- The demo queue accepts up to two requests. The filesystem advances the
  oldest request when the queue is full. Request IDs connect submissions
  and completions. A flush runs after the earlier writes.
- The FTL writes a new physical page on each overwrite and updates the
  logical mapping. Collection moves live pages before erasing an entire
  block. The NAND model has four erase blocks with four pages each.

The device models a flush as an atomic RAM checkpoint of the NAND pages and
logical map. The first cut drops a readable, unflushed append. The second cut
programs the first 37 bytes of a new data page, including two bytes of the
new record's payload. The rest of the page stays erased. The failed program
leaves the checkpoint's mapping at the previous complete page. In both cases
a fresh queue, FTL, and filesystem reopen the surviving image in the same
process and verify the two acknowledged records.

The FTL keeps at least four erased pages for moving live data during
collection. A torn program can use one of those pages. When the surviving
image has fewer than four erased pages, reopening moves live pages out of
an erase block that contains stale pages and erases that block. Both the
live and durable RAM images start from the reclaimed state. A regression
test cuts four consecutive overwrites, checks all committed blocks on each
reopen, then writes and recovers a new value.

## Block requests and geometry

The device validates a contiguous read or write range before enqueueing any
work. A range occupies one queue slot. When it reaches the front, its
4 KiB blocks run in order and share one completion ID. A flush waits for
all blocks in earlier requests. If one block fails, the range completes
with that error and the queue returns cancellation errors for later accepted
requests. The filesystem can acknowledge an append after a successful
sync. Reopening after an error recovers the last flushed image.

An explicit caller submits requests and retrieves their completions.
Filesystem helpers drive their own requests. Switching between these
drivers while requests are pending returns `WouldBlock`. Drain the queue
with the current driver before switching. Both submission and completion
check the driver before advancing work.

`NandGeometry::new(erase_blocks, pages_per_erase_block)` sets the physical
layout. `Ftl::with_geometry(logical_blocks, geometry)` requires room for an
erase block's worth of reserved pages and one extra physical page, beyond
the logical capacity. This ensures the collector can find a stale page at
the write reserve. A cut can spend one of the reserved pages. The stale
page in a candidate block leaves enough erased space elsewhere to move its
live pages and replenish the reserve on reopen.

The tear hook needs a page erased in both the live and durable images. If
earlier unflushed writes leave no shared erased page, the hook reports a
device error before programming any partial bytes.

The [ublk lab](linux/ublk/README.md) connects ext4 to the Rust block device.
The [FUSE lab](linux/fuse/README.md) uses the same device with a root
directory and a metadata journal. It shows how file sync and directory
sync affect recovery.
The [append benchmark](linux/append/README.md) compares synchronous and
io_uring writes with the same commit policy on both filesystems.
The earlier NBD and `fio` pilot is available in Git
history at `073e7f4` ("Fix fio FLUSH and JSON evidence (#2)").
