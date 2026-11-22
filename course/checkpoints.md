# Independent checkpoint tools

These commands prepare the original stand-alone exercises in separate
snapshots. The course chapters use the files in your course checkout.

Run the [append-and-recover demo](../README.md#run-the-demo) first. Follow the
application's two acknowledged records through the queue submissions and NAND
page placements, then find the same records after each cut.

### Open a lab

Run these commands after the [Linux setup](START.md#set-up-linux).
Use a full-history clone so Git and tar can prepare the sources from the
pinned userspace baseline. If you cloned with `--depth`, run
`git fetch --unshallow` first. The prepared sources, Cargo cache, and
build output live on the guest's `/dev/shm` tmpfs.

```sh
cargo xtask checkpoint prepare ftl
```

Copy the directory printed by `prepare`. Substitute that path for
`LAB_DIRECTORY` below.

```sh
cargo xtask checkpoint previous LAB_DIRECTORY
cargo xtask checkpoint check LAB_DIRECTORY
```

`previous` checks the working layer the exercise builds on. The first
`check` shows the missing behavior. Edit the source file listed for your
chapter inside the prepared directory, then run `check` again. Each
prepared lab supplies the completed earlier layers and leaves one change
for you.

### 1. NAND to SSD

The predecessor programs pages and erases whole blocks. Open
`src/ftl.rs` and find the TODO in `Ftl::write`. A write already has a
successful program on an erased page. Publish that page as the LBA's new
location and pass the previous location to the trace. The collector uses
the map to decide which pages it needs to copy.

`check` fills and overwrites the device, triggers a collection that moves
a live page, then checks the new and old values after reopen.

Run the two recovery fixtures after the placement check passes.

```sh
cargo xtask checkpoint faults LAB_DIRECTORY
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
cargo xtask checkpoint prepare queue
```

The predecessor proves the FTL can relocate and recover live blocks. Open
`src/queue.rs` and find the TODO in `complete_next`. Choose which
accepted request to run next so all children of a range submitted before a
flush finish before that flush. Keep a later write outside the flush's
durable image.

`check` fills two queue slots with a three-block write and a flush. It
receives backpressure for a later write, observes the completion IDs, and
reopens the image to read the three earlier blocks.

### 3. Blocks to files

```sh
cargo xtask checkpoint prepare filesystem
```

The predecessor proves the queue's capacity and flush ordering. Open
`src/filesystem.rs` and find the TODO in `append`. The data range is
already queued. Record the file length that covers those bytes in the
directory block so a fresh filesystem opens the appended contents after a
flush. The fixture appends across a 4 KiB boundary and also checks the
capacity error before it reopens.

`check` compares the complete reopened file with the bytes appended before
the flush.

### 4. Append and recover

```sh
cargo xtask checkpoint prepare log
```

The predecessor recovers a file that crosses a block boundary. Open
`src/app.rs` and find the TODO in `decode`. The decoder has the
complete payload and the CRC-32 saved in its header. Check them before
accepting the record and return an invalid-data error for a bad payload.

`check` rejects an incomplete header, an incomplete payload, and a changed
payload. It then runs the append-and-recover demo test to verify the
acknowledged prefix after both simulated cuts.
