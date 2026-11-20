# Measure the guest block queue

Run this from the checkout with fio and Python 3 in the OrbStack Ubuntu
guest. The guest also needs tracefs, udevadm with device locking, and the ublk
setup described in the [ext4 lab](README.md).
This command measures the supplied reference implementation. Use the
[queue chapter](../../course/02-queue.md) to run your learner implementation.

```sh
orb -m ubuntu -u root unshare --mount --propagation private bash linux/ublk/queue-bench.sh
```

The script prints the capture directory on `/dev/shm`. It saves fio JSON,
server timings, the two guest block events, and a Markdown report for the
same measured interval. Each case creates and removes its own device and
tracefs instance.

## Workload

Each case begins with a fresh 32 MiB device, one kernel queue of depth eight,
4 KiB logical blocks, and a write-back cache with `fua=0`. The test uses one
fio job with direct io_uring, mixed random reads and writes, a 70% read
setting, and seed 20261120. The footprint is 16 MiB. A 2 MiB run warms the
device, then a separate job transfers 16 MiB. The timed phase uses
`end_fsync=0`. Both policies use the same Rust release build and dependency
lockfile. The script waits for the owned device to initialize and holds
its udev advisory lock through warmup and capture. [systemd device locking](https://www.freedesktop.org/software/systemd/man/260/udevadm.html#udevadm%20lock%20%5Boptions%E2%80%A6%5D%20command%E2%80%A6)

FIFO runs at 4 KiB with requested depths one and eight, and at 16 KiB with
depth eight. At 4 KiB and depth eight, the first pair runs FIFO then read
priority, and the second runs them in reverse order. Both policies drain
the same kind of bounded libublk completion batch. Read priority can
advance a read within that batch when it is disjoint from earlier writes.
Flushes bound the reordering. The Rust block queue still services one
request at a time. The comparison holds request size, requested depth, seed,
and byte totals fixed.

## Read the measurements

- fio reports total read latency, which includes submission and completion
  latency, plus achieved depth buckets, bytes, elapsed time, and throughput.
  [fio latency and depth reporting](https://fio.readthedocs.io/en/latest/fio_doc.html#cmdoption-arg-lat_percentiles)
- Server queue wait starts when libublk delivers the tag and ends when the
  adapter begins servicing it. Service time covers the simulator request and
  buffer copies. The server counts reads that advanced ahead of other
  requests, plus simulated NAND programs, live copies, and erases.
- The guest trace records block request issue and completion for the owned
  device. The report joins an operation and sector range when exactly one
  issue, completion, and server row have that key. The joined latency uses
  the kernel timestamps. The issue and completion format has no ublk tag,
  so repeated ranges remain outside the joined sample. [Linux event tracing](https://docs.kernel.org/trace/events.html)
- The report checks that guest issue bytes equal successful server bytes in
  each direction and shows any excess over fio, including reads issued by
  udev. These checks also catch gaps in the separately started trace and
  server capture windows.

## Example from OrbStack Ubuntu

The guest ran Linux `7.2.12-orbstack-00394-gbecdd6130068` on arm64 with
Rust 1.93.1, fio 3.41, and udev 259. The first sweep captured 1,048,576 to
1,400,832 extra read bytes on each device, most from udev probes. Waiting
for udev to settle after warmup still left 1,400,832 extra read bytes in
each 4 KiB case. With the per-device advisory lock, all six cases below had
zero extra read or write bytes and zero trace drops.

| Case | Requested depth | fio depth buckets | ms | MiB/s | Read p99 µs |
| --- | ---: | --- | ---: | ---: | ---: |
| 4 KiB FIFO | 1 | 1: 100.0% | 29 | 551.7 | 13.0 |
| 4 KiB FIFO, pair 1 | 8 | 4: 11.5%, 8: 88.5% | 19 | 842.1 | 72.2 |
| 4 KiB read priority, pair 1 | 8 | 8: 100.0% | 17 | 941.2 | 49.4 |
| 4 KiB read priority, pair 2 | 8 | 4: 97.7%, 8: 2.3% | 19 | 842.1 | 73.2 |
| 4 KiB FIFO, pair 2 | 8 | 8: 100.0% | 18 | 888.9 | 76.3 |
| 16 KiB FIFO | 8 | 8: 100.0% | 13 | 1230.8 | 220.2 |

Each 4 KiB case transferred 11,677,696 read and 5,099,520 write bytes.
The 16 KiB case transferred 11,845,632 read and 4,931,584 write bytes.
The guest joined each captured data request in this sweep, including
requests Linux merged before sending them to the server.

| Case | Server read wait / service p99 µs | Reads advanced | Guest joined p99 µs | Guest joins | NAND programs / copies / erases |
| --- | ---: | ---: | ---: | ---: | ---: |
| 4 KiB FIFO, depth 1 | 0.0 / 3.6 | 0 | 11.0 | 4096 | 1245 / 0 / 0 |
| 4 KiB FIFO, pair 1 | 23.9 / 3.9 | 0 | 45.0 | 4075 | 1245 / 0 / 0 |
| 4 KiB read priority, pair 1 | 8.3 / 3.2 | 793 | 39.0 | 4071 | 1245 / 0 / 0 |
| 4 KiB read priority, pair 2 | 10.9 / 3.5 | 867 | 43.0 | 4077 | 1245 / 0 / 0 |
| 4 KiB FIFO, pair 2 | 20.5 / 3.0 | 0 | 43.0 | 4071 | 1245 / 0 / 0 |
| 16 KiB FIFO | 75.0 / 7.7 | 0 | 136.0 | 1010 | 1204 / 0 / 0 |

Read priority shortened server read-wait p99 in both pairs. Fio read p99
fell in both pairs, by 22.8 µs and 3.1 µs. Throughput rose in the first pair
and fell in the second. Earlier locked sweeps moved fio read p99 in opposite
directions. The achieved depth varied across the final 13–29 ms jobs.
These runs demonstrate that the policy changes order and affects wait
inside the server. End-to-end speedup remains unresolved.
The NAND counters describe the simulator and measure no physical SSD I/O.
