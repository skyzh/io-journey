# 2. Blocks in flight

Continue in the checkout with your working FTL from
[NAND to SSD](01-ftl.md). The queue sends its requests to that FTL.

Your inner Rust queue accepts whole requests. A range uses one slot even
when it spans several blocks. Acceptance records an ID. Completion
returns that ID with the result after the device operation runs.

## Admit requests without executing them

Run `cargo xtask learner admit`. The starter accepts one request too
many. In [`course/learner/queue.rs`](https://github.com/skyzh/io-journey/blob/main/course/learner/queue.rs), implement
`has_room` using the pending length and the configured depth. Admission
already validates the whole range. A full queue returns the caller's
submission so the caller can retry after receiving a completion. Run
the check again and verify the rejected request consumed no slot.
The supplied fields are `pending` of type `VecDeque<Request>` and
`depth` of type `NonZeroUsize`.

## Preserve order and IDs

Once admission passes, run `cargo xtask learner order` to check request
order and IDs.
Give each accepted request a distinct ID in `allocate_id`.
Return a rejected submission error if incrementing
`next_id` would overflow. In `take_next_request`, take the oldest
accepted request from the supplied `VecDeque`. Run the check again.
The first two completions must match the submission order, and a
third completion must find the queue empty. `RequestId` is `u64`, and
`SubmitError::Rejected` holds a `std::io::Error`.

## Flush at the request boundary

Run `cargo xtask learner flush`. The check writes a range, submits
a flush, then submits another write as space opens. In `flush_image`,
publish the FTL image when the queue executes that flush. At this point
the earlier range has finished. The supplied `Ftl::flush` takes
`&mut Trace` and returns `()`. The later write stays outside the
durable image. Run the check again to verify the reopened blocks, range
validation, backpressure, and cancellation after a failed block.

## Choose a Linux tag to serve

The ublk adapter plays the device. Its dispatcher chooses among tags
Linux has already sent to it, before forwarding each request to the
Rust device queue. Keep that queue FIFO
and implement the read-first choice in
[`course/learner/dispatch.rs`](https://github.com/skyzh/io-journey/blob/main/course/learner/dispatch.rs).

Once the queue checks pass, run `cargo xtask learner dispatch` to check
policy selection.
FIFO always chooses index zero.
For read-first, find the first read before a barrier that has no
overlapping earlier write. `request` supplies the kind and sector span
for each batch entry. `disjoint` checks both span ends and treats overflow
conservatively. Return zero if no read qualifies. Rerun the dispatch
check for disjoint and overlapping reads, a flush boundary, and overflow.

## Compare on the guest

For the ext4 recovery check, follow the
[ublk prerequisites](https://github.com/skyzh/io-journey/blob/main/linux/ublk/README.md).
From the checkout in the [root Linux shell](START.md#mounted-labs), run:

```sh
cargo xtask ext4 learner
```

The Linux filesystem writes through ublk to your FTL. Check that both
synced records return after the simulated cut.

Use the [fio and tracing prerequisites](https://github.com/skyzh/io-journey/blob/main/linux/ublk/QUEUE_BENCHMARK.md).
Run from the checkout in the [root guest shell](START.md#mounted-labs).

```sh
cargo xtask queue learner
```

The adapter builds your FTL, inner queue, and dispatch from this
checkout. A missing learner module stops the build before a device
is allocated. The two policies share that binary. Read the 4 KiB, depth-eight
pairs before changing request size or depth. Check that Linux block issue
bytes equal the bytes completed successfully by the server. The report
accounts for any excess over fio. Linux completions feed the latency
join. Compare requested depth with
the achieved depth, then compare server queue wait, fio read p99, and
throughput. Reads that advance in the server can still see higher
end-to-end latency if work is batched differently. The guest trace
covers the device and the NAND counters cover the simulator.

A read can wait less at the device and still take longer end to end.
Use the measured queue depth and the guest trace to explain what your
run shows.

Next, [Host block scheduling](02a-scheduler.md) chooses which waiting
request to send before it enters the device queue.
