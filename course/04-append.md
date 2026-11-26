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

The [append adapter](https://github.com/skyzh/io-journey/blob/main/linux/append/src/engine.rs) supplies the Linux
submission plumbing. Each wave has up to four writes at explicit file
offsets. The adapter keeps an `Arc` owner for each submitted buffer as
long as the kernel may still be using it.

Once the receipt check passes, `cargo xtask learner completions` fetches the locked
Linux `io-uring` dependencies into the Cargo cache and runs the completion tests. Open
[`course/learner/append-completions.rs`](https://github.com/skyzh/io-journey/blob/main/course/learner/append-completions.rs).
In `check_completions`, match each returned ID to its request exactly
once and check its byte count. Completion order can vary. Reject
unknown, duplicate, or missing IDs with `InvalidData`, and report a
short write with `WriteZero`. Keep the first I/O failure for a complete
set of IDs.

In `drain_ring_wave`, call the supplied poll function until the wave
has a completion for each request. A short write or failed operation
can arrive while another buffer is still in use. Validate the full
set first. Your code returns the operation result. The supplied adapter
owns the buffer references and proves that the kernel returned one
completion for every submitted ID before it releases them. A complete
ID set retires even if a write returns `InvalidData`. A failed wait
or an invalid ID set keeps the unproven buffers through process exit.
Run the check again to exercise both kinds of failure.

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
