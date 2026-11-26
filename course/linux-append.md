# Linux lab: append on a real kernel

This side chapter runs the append engine from
[Efficient application I/O](04-append.md) on Linux, through the kernel's
`io_uring` interface, on both ext4 and the FUSE filesystem from the
[filesystem lab](linux-filesystem.md). It needs a Linux machine or VM and a
root shell. The main course doesn't depend on it.

## Compare and recover

Follow the [guest prerequisites](https://github.com/skyzh/io-journey/blob/main/linux/append/README.md). Run from the
checkout in the [root Linux shell](START.md#mounted-labs).

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

Compare results within one filesystem before comparing across them. On
ext4 the file writes are buffered in the kernel's page cache, while this
FUSE adapter opens files with direct I/O
([FUSE I/O modes](https://docs.kernel.org/filesystems/fuse/fuse-io.html)),
so the two filesystems do different work for the same append.

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
