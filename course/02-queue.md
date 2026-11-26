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

<a id="choose-a-linux-tag-to-serve"></a>

## Choose a device request to serve

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

The [Linux block-device lab](linux-ublk.md) runs your device queue
through ublk. It includes the ext4 recovery check and a fio comparison.
You can continue to host scheduling on macOS.

Next, [Host block scheduling](02a-scheduler.md) chooses which waiting
request to send before it enters the device queue.
