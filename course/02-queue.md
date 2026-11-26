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

The outer ublk scheduler chooses which kernel tag to send to the inner
Rust queue. It sees a ready batch from Linux. Keep the inner queue FIFO
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

The mounted labs in this chapter build a Linux adapter with a fresh Cargo
cache. Give the build access to the Cargo registry or a configured mirror.

For an optional first look at the kernel using your virtual disk, see the
[ext4 smoke check](https://github.com/skyzh/io-journey/blob/main/linux/ublk/README.md).
It uses Linux’s supplied filesystem.

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

### Recorded comparison

The author's learner build ran six fio cases with equal Linux issue and successful
server byte counts, with zero excess over fio. In each 4 KiB pair below
both arms issued 11,677,696 read bytes
and 5,099,520 write bytes. All four runs requested depth eight.

| Pair and policy | Achieved depth | Reads advanced | Server read wait p99 | fio read p99 | Throughput |
| --- | --- | ---: | ---: | ---: | ---: |
| 1 FIFO | 2 at 0.2%, 4 at 96.9%, 8 at 2.9% | 0 | 11.4 µs | 27.8 µs | 1,600 MiB/s |
| 1 read-first | 2 at 1.4%, 4 at 91.8%, 8 at 6.8% | 881 | 6.0 µs | 42.8 µs | 1,455 MiB/s |
| 2 read-first | 8 at 100% | 793 | 7.4 µs | 32.1 µs | 1,455 MiB/s |
| 2 FIFO | 8 at 100% | 0 | 11.2 µs | 26.8 µs | 1,600 MiB/s |

In this guest sweep the eligible reads advanced and waited less in
the server. End-to-end p99 grew and throughput fell in both pairs.
A separate learner walkthrough also found shorter server wait. Its
fio p99 fell and throughput rose in both pairs, with different
achieved-depth distributions. Compare each sweep using the achieved
depths reported by fio.
In the author's sweep, the depth-one run achieved depth one throughout,
and the 16 KiB run achieved depth eight. The benchmark's report retains
both rows with their request sizes, byte counts, and timings.

Next, [Blocks to files](03-filesystem.md) uses your queue for the
filesystem’s block writes and flushes.
