# Use your queue from Linux

Run the lab after [Blocks in flight](02-queue.md). It uses your NAND,
FTL, and device queue under the Linux ublk driver. The first build
needs access to the Cargo registry or a configured mirror for its
locked Linux dependencies.

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
