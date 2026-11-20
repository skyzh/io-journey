# 2. Blocks in flight

Continue with the same FTL from [NAND to SSD](01-ftl.md). Run
`./course/learner.sh check` before changing the queue. Its predecessor
still has to move live pages and recover committed values.

Your inner Rust queue accepts whole requests. A range uses one slot even
when it spans several blocks. Acceptance records an ID. Completion
returns that ID with the result after the device operation runs.

## Admit requests without executing them

Run `./course/learner.sh admit`. The starter accepts one request too
many. In [`course/learner/queue.rs`](learner/queue.rs), implement
`has_room` using the pending length and the configured depth. Admission
already validates the whole range. A full queue returns the caller's
submission so the caller can retry after receiving a completion. Run
the check again and verify the rejected request consumed no slot.

## Preserve order and IDs

Run `./course/learner.sh order`. Give each accepted request a distinct
ID in `allocate_id`. Return a rejected submission error if incrementing
`next_id` would overflow. In `take_next_request`, take the oldest
accepted request from the supplied `VecDeque`. Run the check again.
The first two completions must match the submission order, and a
third completion must find the queue empty.

## Flush at the request boundary

Run `./course/learner.sh flush`. The fixture writes a range, submits
a flush, then submits another write as space opens. In `flush_image`,
publish the FTL image when the queue executes that flush. At this point
the earlier range has finished. The later write stays outside the
durable image. Run the check again to verify the reopened blocks, range
validation, backpressure, and cancellation after a failed block.

## Choose a Linux tag to serve

The outer ublk scheduler chooses which kernel tag to send to the inner
Rust queue. It sees a ready batch from Linux. Keep the inner queue FIFO
and implement the read-first choice in
[`course/learner/dispatch.rs`](learner/dispatch.rs).

Run `./course/learner.sh dispatch`. FIFO always chooses index zero.
For read-first, find the first read before a barrier that has no
overlapping earlier write. `request` supplies the kind and sector span
for each batch entry. `disjoint` checks both span ends and treats overflow
conservatively. Return zero if no read qualifies. Run the check again.
It tests disjoint and overlapping reads, a flush boundary, and overflow.

## Compare on the guest

Use the [fio and tracing prerequisites](../linux/ublk/QUEUE_BENCHMARK.md)
and run from the Mac checkout shell. The Linux command starts at
`unshare` and runs as guest root.

```sh
orb -m ubuntu -u root unshare --mount --propagation private bash linux/ublk/queue-bench.sh learner
```

The adapter builds your FTL, inner queue, and dispatch from this
checkout. The two policies share that binary. Read the 4 KiB, depth-eight
pairs before changing request size or depth. Check that fio, Linux block
completion, and server byte counts agree. Compare requested depth with
the achieved depth, then compare server queue wait, fio read p99, and
throughput. Reads that advance in the server can still see higher
end-to-end latency if work is batched differently. The guest trace
covers the device and the NAND counters cover the simulator.
