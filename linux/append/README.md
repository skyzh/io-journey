# Compare durable append engines

Run from the checkout in the [root guest shell](../../course/START.md#mounted-labs).
The lab uses the [ublk](../ublk/README.md) and [FUSE](../fuse/README.md)
setup and Python 3.

```sh
cargo xtask append
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
after this check and mount cleanup succeed. If an owned mount stays busy,
the backing server and device stay live until it is unmounted.

## Example from OrbStack Ubuntu

Linux `7.2.12-orbstack-00394-gbecdd6130068` ran on arm64 with Rust 1.93.1,
udev 259, io-uring 0.7.16, and fuser 0.16.0. All 16 cases recovered the
saved prefix. The timed intervals ranged from 6.10 ms to 40.07 ms.

| Filesystem | Group | Run | Engine | Records/s | Ack p50 µs | Ack p99 µs | Saved FTL flush |
| --- | ---: | ---: | --- | ---: | ---: | ---: | ---: |
| ext4 | 4 | 1 | sync | 7138.9 | 559.2 | 608.4 | 166 |
| ext4 | 4 | 2 | uring | 6596.2 | 598.4 | 715.9 | 166 |
| ext4 | 4 | 3 | uring | 6388.8 | 612.9 | 741.7 | 166 |
| ext4 | 4 | 4 | sync | 6719.6 | 597.9 | 667.0 | 166 |
| ext4 | 32 | 1 | sync | 33802.5 | 899.0 | 1094.1 | 26 |
| ext4 | 32 | 2 | uring | 35525.8 | 906.3 | 968.3 | 26 |
| ext4 | 32 | 3 | uring | 41951.8 | 728.5 | 889.4 | 26 |
| ext4 | 32 | 4 | sync | 41169.8 | 763.5 | 822.7 | 26 |
| FUSE | 4 | 1 | sync | 8317.4 | 489.2 | 556.1 | 406 |
| FUSE | 4 | 2 | uring | 8068.0 | 497.2 | 561.7 | 406 |
| FUSE | 4 | 3 | uring | 7563.6 | 545.2 | 602.9 | 406 |
| FUSE | 4 | 4 | sync | 8171.3 | 493.3 | 566.2 | 406 |
| FUSE | 32 | 1 | sync | 28523.7 | 1092.2 | 1483.3 | 56 |
| FUSE | 32 | 2 | uring | 33877.8 | 951.6 | 1004.8 | 56 |
| FUSE | 32 | 3 | uring | 30151.9 | 1014.3 | 1173.5 | 56 |
| FUSE | 32 | 4 | sync | 32205.5 | 983.9 | 1166.5 | 56 |

Groups of 32 increased throughput and median acknowledgement latency
on both filesystems. In this sweep, io_uring throughput was lower in all
four pairs at group size 4. At size 32, it was higher in both ext4 pairs
and moved in opposite directions on FUSE. An earlier sweep saw mixed
ext4 results at size 32 and lower io_uring throughput in both FUSE pairs.
The short intervals and the eight groups per run at size 32 limit tail
latency conclusions. These measurements describe the guest and simulated
device.
