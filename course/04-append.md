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
its group's sync, so acknowledgement latency can rise too. The lab at the
end measures whether it does.

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

## Compare and recover

Follow the [guest prerequisites](https://github.com/skyzh/io-journey/blob/main/linux/append/README.md). Run from the
checkout in the [root guest shell](START.md#mounted-labs).

```sh
cargo xtask append learner
```

One release build selects the learner append code for the synchronous
and io_uring engines. Both use the same offsets, record bytes, four
requests per wave, and sync policy. Each fresh device takes 64 warmup
and 256 measured records.

The runner captures the acknowledgement count and the exact FTL flush
before writing the visible unacknowledged tail. It removes the mount,
reopens that saved image, and remounts. The recovered file must have
exactly the bytes covered by the receipt, in order and with valid
checksums. This checks the kernel ext4 path and the learner FUSE
filesystem backed by the learner queue and FTL.

The lab answers two questions. Does every acknowledged record survive a
power cut? And how do group size and engine change throughput and
acknowledgement latency?

For recovery, each case should print `verified all 320 acknowledged
records`. A failure there is a correctness bug in your code, not a
performance result.

For performance, compare the `Records/s` and `Ack p50 µs` columns in the
report:

- **Group size.** Within one filesystem and engine, compare groups of 4
  and 32. Check whether throughput rises at 32 and whether p50 latency
  rises with it. Latency doesn't always move the same way in every run.
- **Engine.** Within one filesystem and group size, compare `sync` with
  `uring` (io_uring). The lab runs each engine twice per case. The size
  and direction of the difference can vary between runs, so compare both
  repeats before drawing a conclusion.

## Follow the acknowledgement across layers

Pick one case from the lab output. Three printed lines connect the
application's receipt to the flash:

1. `Application acknowledged 320 records through file sync.` This is the
   receipt: 64 warmup and 256 measured records.
2. `ublk tag … request … flush … published FTL checkpoint`. After the
   application reported its receipt, and before it wrote the
   unacknowledged tail, the runner saved the latest durable FTL image.
   This line is the flush that published that image. It names the Linux
   kernel tag, the request ID in your Rust queue, and the FTL checkpoint
   number.
3. `Reopened FTL flush … and verified all 320 acknowledged records.` After
   the cut, the FTL reopened that same checkpoint and the file still held
   every acknowledged record.

Every record is 512 bytes, and IDs start at 0. So record *n* starts at byte
offset *n* × 512. The last acknowledged record, ID 319, starts at offset
163,328, and the receipt covers 320 × 512 = 163,840 bytes.

You can't follow one record to one kernel tag or one NAND page. The
filesystem decides where its blocks go, writes back in the background, and
the kernel can split or merge requests. The link you can check end to end
is the sequence. The engine acknowledged each record only after the file
sync covering it returned. The runner then saved the latest durable
checkpoint, and recovery from that checkpoint returned every acknowledged
byte. To see
individual NAND programs and mapping updates, use the demo trace instead.
