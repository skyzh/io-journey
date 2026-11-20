# Compare durable append engines

Run from the checkout in the OrbStack Ubuntu guest. The script uses the
[ublk](../ublk/README.md) and [FUSE](../fuse/README.md) setup, Python 3,
and a private mount namespace.

```sh
orb -m ubuntu -u root unshare --mount --propagation private bash linux/append/benchmark.sh
```

The script prints its capture directory on guest tmpfs. It first runs on
ext4, then on the learner filesystem through FUSE. Each case starts with
a fresh, verified 32 MiB ublk device with a write-back cache and separate
flushes to the RAM-backed FTL. The build and capture files live in tmpfs.

## Trace a durable receipt

Both engines publish the file's name with a directory sync before
running. They write the same 512-byte CRC-framed records at explicit byte
offsets without `O_APPEND`. One engine uses four persistent workers making
blocking positional writes. The other submits up to four writes together
to an io_uring. [Linux positional writes](https://man7.org/linux/man-pages/man2/pwrite.2.html) · [io_uring submissions and completions](https://man7.org/linux/man-pages/man2/io_uring_enter.2.html)

Each wave keeps its buffers until every submitted write completes.
The engine validates the completion IDs and byte counts before the group
can call `sync_all`. An error or short write leaves the group
unacknowledged. If the ring cannot finish collecting a wave, the CLI
keeps those buffers alive until exit. Both engines issue the same file
sync and acknowledge the group only when it succeeds.

The workload appends 64 warmup records and then times 256 records
for 128 KiB per case. Each group size runs two engine pairs in reversed
order. Four writes are admitted per wave in every case. Group size 4
syncs after one wave, and group size 32 syncs after eight waves. This
changes the commit interval at the same concurrency bound.

Each acknowledgement latency starts when the group is ready and ends at
successful file sync. It includes waits behind earlier waves in the group.
Records in one group share a latency sample. Throughput uses the entire
timed interval, and p50 and p99 use nearest ranks. Each case measures 64
groups at size 4 and eight groups at size 32.

After the last receipt, the script saves the FTL checkpoint. It writes
and reads back four more records without syncing, unmounts, and restores
the saved image. The remount checks the exact 320-record prefix including
the warmup, with CRC and sequence validation. A row is reported only
after this check and mount cleanup succeed.

## Example from OrbStack Ubuntu

Linux `7.2.12-orbstack-00394-gbecdd6130068` ran on arm64 with Rust 1.93.1,
udev 259, io-uring 0.7.16, and fuser 0.16.0. All 16 cases recovered the
saved prefix. The timed intervals ranged from 6.66 ms to 48.47 ms.

| Filesystem | Group | Run | Engine | Records/s | Ack p50 µs | Ack p99 µs | Saved FTL flush |
| --- | ---: | ---: | --- | ---: | ---: | ---: | ---: |
| ext4 | 4 | 1 | sync | 7876.7 | 505.2 | 574.2 | 166 |
| ext4 | 4 | 2 | uring | 6552.4 | 610.8 | 701.0 | 166 |
| ext4 | 4 | 3 | uring | 6672.0 | 593.5 | 698.0 | 166 |
| ext4 | 4 | 4 | sync | 6381.8 | 625.2 | 735.5 | 166 |
| ext4 | 32 | 1 | sync | 33879.4 | 945.0 | 1033.1 | 26 |
| ext4 | 32 | 2 | uring | 34396.3 | 912.0 | 978.6 | 26 |
| ext4 | 32 | 3 | uring | 33717.3 | 929.6 | 1011.5 | 26 |
| ext4 | 32 | 4 | sync | 38427.9 | 845.0 | 879.5 | 26 |
| FUSE | 4 | 1 | sync | 8144.8 | 500.3 | 567.8 | 406 |
| FUSE | 4 | 2 | uring | 5281.7 | 725.1 | 1411.4 | 406 |
| FUSE | 4 | 3 | uring | 8042.9 | 498.5 | 560.8 | 406 |
| FUSE | 4 | 4 | sync | 8039.0 | 503.0 | 566.4 | 406 |
| FUSE | 32 | 1 | sync | 28959.1 | 1098.9 | 1306.5 | 56 |
| FUSE | 32 | 2 | uring | 23526.3 | 1091.4 | 2088.2 | 56 |
| FUSE | 32 | 3 | uring | 29415.0 | 1073.1 | 1149.0 | 56 |
| FUSE | 32 | 4 | sync | 30223.6 | 1034.5 | 1152.7 | 56 |

Groups of 32 increased throughput and median acknowledgement latency
on both filesystems. The engine comparisons varied across repeats. For
example, io_uring was slower in both pairs at group size 32 on FUSE and
had mixed throughput on ext4. The short intervals, especially the eight
groups per run at size 32, limit conclusions about tail latency. These
measurements describe the guest and simulated device.
