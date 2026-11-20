# I/O Journey

Follow an append from a Rust application through a filesystem, a bounded
block queue, and a flash translation layer (FTL) to simulated NAND. The demo
flushes two records and checks their recovery after two simulated power cuts.

## Run the demo

From the checkout on a Mac with an OrbStack machine named `ubuntu`, run the
following with Rust and Cargo installed in the guest. It uses `/dev/shm` for
Cargo's cache, build output, and temporary files. The prototype has no
third-party dependencies.

```sh
orb -m ubuntu bash -eu -c '
  scratch=$(mktemp -d /dev/shm/io-journey.XXXXXX)
  trap "rm -rf -- \"$scratch\"" EXIT
  mkdir "$scratch/tmp"
  export CARGO_HOME="$scratch/cargo"
  export CARGO_TARGET_DIR="$scratch/target"
  export TMPDIR="$scratch/tmp"
  export CARGO_INCREMENTAL=0
  cargo run --locked --offline --quiet
'
```

On Linux, run the same Bash body directly. For focused tests, use
`cargo test --locked --offline` as the last line of the body. The demo and tests
were verified on OrbStack Ubuntu 26.04.1 arm64 with Rust 1.93.1.

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

## Follow a record

- The application writes a length, a CRC-32, and the payload. Recovery reads
  and checks every complete record in the file.
- The filesystem has one root entry, `/events.log`. Logical block 0 stores
  its length. Seven data blocks store the contents. An append queues data
  writes before the length update.
- The queue accepts up to two 4 KiB requests. At capacity it completes the
  oldest request before accepting another. Request IDs connect submissions and
  completions. A flush runs after the earlier writes.
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

The course follow-ups cover metadata journaling and Linux integration through
ublk, FUSE, and io_uring. The earlier NBD and `fio` pilot is available in Git
history at `073e7f4` ("Fix fio FLUSH and JSON evidence (#2)").
