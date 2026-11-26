# 2. Blocks in flight

Continue in the checkout with your working FTL from
[NAND to SSD](01-ftl.md). The queue sends its requests to that FTL.

## What the queue does

The FTL from Chapter 1 runs one call at a time, as soon as it is made. A
real device accepts several requests, holds them, and runs them while new
ones keep arriving. This chapter adds that layer: a device queue in front
of your FTL.

- **Submit.** A caller hands the queue a request: a read or write of a
  range of blocks, or a flush. If there is room, the queue accepts it and
  returns an ID. A range takes one slot, however many blocks it covers.
- **Complete.** The queue takes the oldest accepted request, runs it
  against the FTL, and returns its ID with the result.
- **Depth.** The queue holds at most a fixed number of accepted requests
  that haven't completed yet.
- **Backpressure.** When the queue is full, submit hands the request back
  instead of accepting it. The caller has to collect a completion, which
  frees a slot, before trying again. The demo trace shows this as
  `backpressure at depth 2. Completing the oldest request`.

A flush is a request too. Your writes change the FTL's live mapping, which
a power cut loses, just like a real drive's volatile write cache. When the
queue runs a flush, the FTL publishes its durable image, as in
[What a flush saves](01-ftl.md#what-a-flush-saves). That image covers every
write the queue completed before the flush. This queue completes requests
in submission order, so a flush covers exactly the writes submitted before
it. Writes submitted after it aren't covered until the next flush.

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

A device doesn't have to serve waiting requests in arrival order. When
several requests have reached the device but haven't entered its FIFO
queue yet, the device's dispatcher picks which one to admit next. For example, it can let a read go ahead of an earlier write, so the
read isn't stuck behind slower work. It must never let a read pass a write
to the same blocks, because the read would return stale data. It must
never move anything across a flush.

Keep the device queue itself FIFO. Implement the choice as a separate
function, `next_index` in
[`course/learner/dispatch.rs`](https://github.com/skyzh/io-journey/blob/main/course/learner/dispatch.rs).
It receives a policy, the batch of waiting requests, and a closure that
gives each request's kind and sector span. It returns the index of the
request to serve next.

Once the queue checks pass, run `cargo xtask learner dispatch` to check
policy selection.

- **FIFO** always chooses index zero.
- **Read-first** chooses the first read that comes before any barrier (a
  flush, `Kind::Barrier`) and
  doesn't overlap an earlier write in the batch. The supplied `disjoint`
  checks both span ends and treats overflow conservatively. Return zero
  if no read qualifies.

Rerun the dispatch check for disjoint and overlapping reads, a flush
boundary, and overflow. In the [Linux block-device lab](linux-ublk.md),
the ublk adapter uses this same function to choose among the request tags
Linux has already sent it.

<a id="compare-on-the-guest"></a>

## Try it on Linux

The optional [Linux block-device lab](linux-ublk.md) runs your device
queue through ublk. It includes an ext4 recovery check and a fio
comparison of FIFO and read-first. The main path continues on macOS or
Linux.

Next, [Host block scheduling](02a-scheduler.md) chooses which waiting
request to send before it enters the device queue.
