# Schedule block requests on the host

You have a FIFO device queue that accepts block requests and returns
completions. Add a scheduler in front of it. While the device has no
free slot, the host can keep requests ready and choose which one to
send next. Linux has a similar separation between software staging and
hardware dispatch, although it has multiple queues and device
completions can arrive out of order. With Linux `none`, the block layer
still queues and dispatches requests without an I/O scheduler reordering
them. [Linux multiqueue block layer](https://docs.kernel.org/block/blk-mq.html)

The preceding [device dispatch exercise](02-queue.md#choose-a-device-request-to-serve)
selected among requests that had reached the device. Here your policy runs before requests enter the
device queue. Once a request enters that queue, the device completes
the work in FIFO order.

## Compare two dispatch orders

From the course checkout, run:

```sh
cargo xtask host-schedule
```

The model gives the device two queue slots and one service engine. All
eight requests arrive at tick zero. Both policies use the same service
costs: 2 ticks for a read, 8 for a write, and 4 for a flush.

| Request | Operation |
| --- | --- |
| 1 | Write `0x12` to LBA 0 |
| 2 | Read LBA 0 |
| 3, 4 | Read LBAs 1 and 2 |
| 5 | Flush |
| 6 | Overwrite LBA 0 with `0x34` |
| 7 | Read LBA 3 |
| 8 | Read LBA 0 |

Pass-through sends the oldest staged request first. Bounded read-first
can advance one read past writes before it returns to the oldest
request. Request 3 can advance ahead of request 1 because their
ranges do not overlap. Request 2 needs the bytes from request 1.
Request 7 can advance ahead of request 6 after the flush, while
request 8 still needs request 6.

## Implement the host policy

Run the focused check, then open
[`course/learner/scheduler.rs`](https://github.com/skyzh/io-journey/blob/main/course/learner/scheduler.rs).

```sh
cargo xtask learner scheduler
```

The starter sends both policies in arrival order. In `choose_request`,
return index zero for pass-through or when `remaining_bypasses` is
zero. Otherwise use `crate::dispatch::next_index` with
`crate::dispatch::Policy::ReadFirst`, the staged slice, and a closure
that returns a staged request's `request` field. This reuses your range
and flush rules from the device-side exercise.

In `after_dispatch`, decrement the allowance for a read selected from
an index other than zero. Keep it unchanged for a read at index zero.
Reset it to `super::READ_BYPASS_BUDGET` when a write or barrier is sent.
The driver supplies the dispatched request's index and kind and
associates the device completion ID with the host request.

Rerun the focused check. It verifies the order and read results at
device depths one and two. It also simulates a cut after the trace.
The later overwrite was visible, but only the write before the flush
survives.

```sh
cargo xtask host-schedule learner
```

Compare how long requests 1 and 3 wait before the device starts them.
The read advances and the older write waits longer. Both policies
finish at tick 30 because they run the same work through one busy
service engine. These are modeled ticks. To observe timing and queue
depth through an actual Linux driver, see the
[Linux block-device lab](linux-ublk.md).

The [planned device labs](planned-labs.md) list the QEMU, SPDK, and
VM virtio paths for future comparisons.

Next, [Blocks to files](03-filesystem.md) uses block writes and
flushes to publish changes to a filesystem.
