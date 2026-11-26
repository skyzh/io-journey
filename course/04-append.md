# 4. Efficient application I/O

Keep your working FTL, queue, and journal. This chapter builds an append engine
that issues bounded positional writes and acknowledges a group only
after successful file sync.

## Why acknowledge only after sync

An application that tells its client "saved" makes a promise: that record
will still be there after a crash. A successful `write` doesn't keep that
promise. The bytes may sit in the kernel's page cache or the device's
volatile cache. Only a successful file sync (`fsync`) waits until the
filesystem has committed them and the device has flushed. So the engine
acknowledges a record only after the sync that covers it returns.

Syncing after every record is safe but slow, because each sync waits for
a device flush. Group commit writes several records, syncs once, then
acknowledges all of them together. Larger groups need fewer syncs per
record, so throughput should rise. Each record may also wait longer for
its group's sync, so acknowledgement latency can rise too. The
[Linux append lab](linux-append.md) measures whether it does.

File sync only covers the file's data and length. The file's name is a
directory entry, which, as in [Blocks to files](03-filesystem.md), only
becomes durable when the directory is synced. So the runner syncs the
parent directory once, right after it creates the log file and before any
appends.

The engine records what it acknowledged as a receipt: the number of bytes
at the front of the file that are durable. After a power cut, those bytes
must come back exactly. Anything after them may or may not survive.

## Recover the bytes covered by a receipt

Each record has a length, a checksum, and its payload. A receipt gives
the byte count the app already acknowledged. During the lab, the
runner holds that receipt outside the file it cuts.

Run `cargo xtask learner receipt` and open [`course/learner/record.rs`](https://github.com/skyzh/io-journey/blob/main/course/learner/record.rs). Limit the supplied
`decode` function to the exact length in the receipt. Return an
`InvalidData` error if the receipt extends beyond the bytes available.
A partial record after the receipt does not change the durable prefix.
A damaged header, missing byte, or wrong checksum inside the receipt
is an error. Run the check again to see both cases.

## Retire a bounded wave

Run the supplied mock from the checkout on macOS or Linux:

```sh
cargo xtask mock-ring
```

The program submits two writes to different offsets. It fetches both
requests in order, then completes request 2 before request 1. The
result is `abcdef` because each write specifies its own offset.
The submission and completion queues have separate bounds. A full
submission queue applies backpressure to the caller. A full
completion queue must be drained before the mock finishes more work.

Run `cargo xtask learner completions` and open
[`course/learner/append-completions.rs`](https://github.com/skyzh/io-journey/blob/main/course/learner/append-completions.rs).
The tests check queue capacity and reach the missing completion
validation and wave retirement.

In `check_completions`, match each returned ID to its request exactly
once and check its byte count. Reject unknown, duplicate, or missing IDs
with `InvalidData`. Report a short write with `WriteZero` and keep the
first I/O failure for a complete set of IDs.

In `drain_ring_wave`, call the supplied poll function until the wave
has a completion for each request. A short write or failed operation
can arrive before the other writes finish. Collect the complete set
before returning its result. Rerun the check and then see your code
drive the mock with `cargo xtask mock-ring learner`.

The [Linux append lab](linux-append.md) runs these same checks against
completions from the real kernel engine.

## Acknowledge after sync

When the completion checks pass, run `cargo xtask learner ack`. The group check reaches
`implement group acknowledgement`. Open
[`course/learner/append-commit.rs`](https://github.com/skyzh/io-journey/blob/main/course/learner/append-commit.rs) and write
the group in chunks of `DEPTH`. Once all waves in the group succeed,
sync the file once. Add one receipt per record only after that sync
succeeds. Each
latency runs from the start of the group to that point.

Rerun the group check. A failed or short write cannot reach sync or
grow the receipts. Failure to sync leaves the entire group without
new receipts. Then run `cargo xtask test` to check the whole stack together.

## Try it on Linux

The [Linux lab: append on a real kernel](linux-append.md) runs the same
append engine against Linux's `io_uring` and both ext4 and your FUSE
filesystem. It simulates a power cut by reopening a saved device
checkpoint, and checks that every acknowledged record comes back.
