# 4. Efficient application I/O

Keep your working FTL, queue, and journal. Start with
`cargo xtask learner replay`. This chapter builds an append engine
that issues bounded positional writes and acknowledges a group only
after successful file sync.

## Recover the bytes covered by a receipt

Each record has a length, a checksum, and its payload. A receipt gives
the byte count the app already acknowledged. During the lab, the
runner holds that receipt outside the file it cuts.

Run `cargo xtask learner receipt` and open
[`course/learner/record.rs`](learner/record.rs). Limit the supplied
`decode` function to the exact length in the receipt. Return an
`InvalidData` error if the receipt extends beyond the bytes available.
A partial record after the receipt does not change the durable prefix.
A damaged header, missing byte, or wrong checksum inside the receipt
is an error. Run the check again to see both cases.

## Retire a bounded wave

The [append adapter](../linux/append/src/engine.rs) supplies the Linux
submission plumbing. Each wave has up to four writes at explicit file
offsets. The adapter keeps an `Arc` owner for each submitted buffer as
long as the kernel may still be using it.

Run `cargo xtask learner completions`. This step fetches the locked
Linux `io-uring` dependency into tmpfs. Open
[`course/learner/append-completions.rs`](learner/append-completions.rs).
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

Run `cargo xtask learner ack`. Open
[`course/learner/append-commit.rs`](learner/append-commit.rs) and write
the group in chunks of `DEPTH`. Once all waves in the group succeed,
sync the file once. Add one receipt per record only after that sync
succeeds. Each
latency runs from the start of the group to that point.

Run the check again. A failed or short write cannot reach sync or
grow the receipts. Failure to sync leaves the entire group without
new receipts.

## Compare and recover

Follow the [guest prerequisites](../linux/append/README.md). Run from the
checkout in the [root guest shell](START.md#mounted-labs).

```sh
cargo xtask append learner
```

One release build selects the learner append code for the synchronous
and io_uring engines. Both use the same offsets, record bytes, four
requests per wave, and sync policy. Each fresh device takes 64 warmup
and 256 measured records. Compare engines within a filesystem and a
group size. Then compare groups of 4 and 32 to see how acknowledgement
latency changes with throughput. Keep both results when repetitions
disagree.

The runner captures the acknowledgement count and the exact FTL flush
before writing the visible unacknowledged tail. It removes the mount,
reopens that saved image, and remounts. The recovered file must have
exactly the bytes covered by the receipt, in order and with valid
checksums. This checks the kernel ext4 path and the learner FUSE
filesystem backed by the learner queue and FTL.

In one guest sweep every case recovered all 320 acknowledged records.
Each cell below gives the two observations for its engine. The raw
runner report also includes p99 and the saved checkpoint for each run.

| Filesystem and group | Engine | Records per second | Acknowledgement p50, µs |
| --- | --- | --- | --- |
| ext4, 4 | sync | 6841, 5883 | 584, 670 |
| ext4, 4 | io_uring | 5974, 6158 | 667, 652 |
| ext4, 32 | sync | 40637, 49237 | 764, 654 |
| ext4, 32 | io_uring | 39249, 46168 | 826, 689 |
| FUSE, 4 | sync | 7847, 8147 | 513, 431 |
| FUSE, 4 | io_uring | 7298, 7535 | 541, 518 |
| FUSE, 32 | sync | 29557, 39817 | 1104, 748 |
| FUSE, 32 | io_uring | 27632, 41246 | 1179, 748 |

Larger groups increased throughput in every matched repeat. The
acknowledgement p50 usually rose. The second ext4 sync repeat went
from 670 to 654 µs. The engine comparison varied with filesystem,
group size, and repetition. Both group-32 FUSE engines also had
substantially different rates between their two runs.

## Follow the acknowledgement across layers

Use one case's receipt and the printed ublk flush. The app writes a
known record at its explicit byte offset. File sync reaches ext4 or
the FUSE journal. The FUSE route publishes metadata through its redo
and marker barriers. The ublk log names the kernel tag, the inner
Rust request ID, and the FTL checkpoint number at the observed flush.
That checkpoint retains the NAND pages and map that the reopen uses.
The final count and checksums connect the recovered file to the
receipt.

An application offset is meaningful in the file, and a kernel sector
range is meaningful on the device. Filesystem allocation, writeback,
and request splitting or merging prevent joining each record to a
single kernel tag or NAND page. Use the observed flush boundary and
the recovery result for the end-to-end check. Use the fio lab for
measured guest block events and the demo for individual NAND program
and map events.

In the first FUSE synchronous group-4 run, the final acknowledged
record had ID 319 and offset 163,328. The total receipt was 163,840
bytes. The runner printed ublk tag 5, inner request 1,374, and FTL
flush 406 for the captured checkpoint. Reopening that checkpoint
verified the same 320 records. The observed link is the receipt
and flush. The physical page holding record 319 stays unknown.
