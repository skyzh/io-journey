# Use your queue from Linux

Run this lab after [Blocks in flight](02-queue.md). It puts your simulated
device behind the Linux ublk driver, so Linux sees it as an ordinary block
device. It has two parts, and each exercises different code of yours:

- `cargo xtask ext4 learner` uses your NAND and FTL from Chapter 1.
- `cargo xtask queue learner` also uses your device queue and dispatch
  policy from Chapter 2.

Both need a root Linux shell (see [mounted labs](START.md#mounted-labs)).
The first build needs access to the Cargo registry or a configured mirror
for its locked Linux dependencies.

## Recover ext4 on your device

Follow the [ublk prerequisites](https://github.com/skyzh/io-journey/blob/main/linux/ublk/README.md),
then run from the checkout in the root Linux shell:

```sh
cargo xtask ext4 learner
```

Linux's ext4 filesystem writes through ublk to your FTL. Check that both
synced records come back after the simulated cut.

## Compare FIFO and read-first with fio

Follow the [fio and tracing prerequisites](https://github.com/skyzh/io-journey/blob/main/linux/ublk/QUEUE_BENCHMARK.md),
then run from the checkout in the root Linux shell:

```sh
cargo xtask queue learner
```

The adapter builds your FTL, device queue and dispatch from this checkout,
and runs both policies from the same binary. If a learner module is
missing, the build stops before a device is allocated. Then read the
report in this order:

1. **Check the bytes.** Linux block issue bytes should equal the bytes the
   server completed successfully. The report accounts for any excess over
   fio.
2. **Start with the 4 KiB, depth-eight pairs** before looking at other
   request sizes or depths.
3. **Compare requested and achieved depth.** fio may not keep eight
   requests in flight.
4. **Compare server queue wait, fio read p99 and throughput** between FIFO
   and read-first.

A read can wait less at the device and still take longer end to end, if
the work is batched differently. The guest trace covers the device, and
the NAND counters cover the simulator. Use the measured depth and the
trace to explain what your run shows.
