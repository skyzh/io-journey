# I/O Journey chapters

[Start here](START.md) for setup and the first run. The cumulative chapters
carry your changes forward in one checkout.

1. [NAND to SSD](01-ftl.md). Map writes, reclaim a block, and recover an ext4 log using your FTL.
   Optional extensions cover [free-space tracking](01a-ftl-index.md) and [shared checkpoint pages](01b-ftl-snapshots.md).
   The [NAND timing lab](01c-nand-latency.md) shows read contention and garbage collection in virtual time.
2. [Blocks in flight](02-queue.md). Add bounded requests, flush ordering, and Linux tag dispatch.
3. [Blocks to files](03-filesystem.md). Publish file and directory changes and replay them through FUSE.
4. [Efficient application I/O](04-append.md). Recover a receipt and compare durable append engines.

## Independent checkpoints

These are optional exercises in separate snapshots. Use the cumulative
chapters above to carry your implementation forward in the course checkout.

Run the [append-and-recover demo](../README.md#run-the-demo) first. Follow the
application's two acknowledged records through the queue submissions and NAND
page placements, then find the same records after each cut.

### Open a lab

Run these commands from the checkout in an OrbStack Ubuntu shell or another
Linux shell with Bash, Git, tar, findmnt, and Rust. The Git history supplies
the pinned userspace baseline. The prepared sources, Cargo cache, and build
output live on the guest's `/dev/shm` tmpfs.

```sh
lab=$(./course/checkpoint.sh prepare ftl)
./course/checkpoint.sh previous "$lab"
./course/checkpoint.sh check "$lab"
```

`previous` checks the working layer the exercise builds on. The first
`check` shows the missing behavior. Edit the source file listed for your
chapter inside `$lab`, then run `check` again. Each prepared lab supplies
the completed earlier layers and leaves one change for you.

### 1. NAND to SSD

The predecessor programs pages and erases whole blocks. Open
`$lab/src/ftl.rs` and find the TODO in `Ftl::write`. A write already has a
successful program on an erased page. Publish that page as the LBA's new
location and pass the previous location to the trace. The collector uses
the map to decide which pages it needs to copy.

`check` fills and overwrites the device, triggers a collection that moves
a live page, then checks the new and old values after reopen.

Run the two recovery fixtures after the placement check passes.

```sh
./course/checkpoint.sh faults "$lab"
```

The first fixture has a page erased in both RAM images. The hook programs
19 bytes of that page and leaves an erased suffix. The durable map still
leads to the earlier complete value. The second fixture uses four
one-page erase blocks and two logical blocks. Earlier live writes leave the
images with erased pages at different addresses, so the hook fails before
programming any partial bytes. Reopen recovers the previous values in both
fixtures.

### 2. Blocks in flight

```sh
lab=$(./course/checkpoint.sh prepare queue)
```

The predecessor proves the FTL can relocate and recover live blocks. Open
`$lab/src/queue.rs` and find the TODO in `complete_next`. Choose which
accepted request to run next so all children of a range submitted before a
flush finish before that flush. Keep a later write outside the flush's
durable image.

`check` fills two queue slots with a three-block write and a flush. It
receives backpressure for a later write, observes the completion IDs, and
reopens the image to read the three earlier blocks.

### 3. Blocks to files

```sh
lab=$(./course/checkpoint.sh prepare filesystem)
```

The predecessor proves the queue's capacity and flush ordering. Open
`$lab/src/filesystem.rs` and find the TODO in `append`. The data range is
already queued. Record the file length that covers those bytes in the
directory block so a fresh filesystem opens the appended contents after a
flush. The fixture appends across a 4 KiB boundary and also checks the
capacity error before it reopens.

`check` compares the complete reopened file with the bytes appended before
the flush.

### 4. Append and recover

```sh
lab=$(./course/checkpoint.sh prepare log)
```

The predecessor recovers a file that crosses a block boundary. Open
`$lab/src/app.rs` and find the TODO in `decode`. The decoder has the
complete payload and the CRC-32 saved in its header. Check them before
accepting the record and return an invalid-data error for a bad payload.

`check` rejects an incomplete header, an incomplete payload, and a changed
payload. It then runs the append-and-recover demo test to verify the
acknowledged prefix after both simulated cuts.
