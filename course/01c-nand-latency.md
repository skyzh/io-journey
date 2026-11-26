# Read latency under NAND contention

From a clean checkout, complete [the setup](START.md#get-the-course)
in the Linux guest and run this from the repository root. The lab uses
the reference FTL and is ready before you start any chapter work.

```sh
cargo xtask nand-latency
```

All NAND pages and simulated events are held in RAM. The default virtual
service times are 80 µs for a read, 400 µs for a program, and 1,600 µs for
an erase. They are illustrative. Append `--read-us`, `--program-us`, or
`--erase-us` followed by a positive integer to explore different service times.

Each row offers 48 reads of 4 KiB, one pair every 200 µs. The pairs address
the same prewritten LBAs, 0 and 4, on dies 0 and 1 respectively. The mixed
rows also offer 24 overwrites of LBA 8, one 25 µs before each read pair.
The same 12 logical pages are written before the clock starts in each row.

The first two rows have 40 physical pages, leaving 28 erased pages after
preconditioning. The third has 20, leaving eight erased pages. Only die 0
gets the extra blocks. Die 1 holds the cold block containing LBA 4, and
all measured writes and collection operations use die 0 in both mixed
rows. The FTL collects when only four erased pages remain. The collector
accounts for each relocated page with its actual source read and
destination program, then erases the victim before the host program.
`GC copy` counts relocated pages and `GC pgm` counts their destination
programs.

Compare the first two rows to see how a program makes a read wait on the
same die. Compare the two mixed rows to see the additional delay from
copies and erases. The trace shows the first collection alongside the
corresponding read pair. It omits the host program, which runs from
4,000 to 4,400 µs at the default service times. Die 1 can service its read
while die 0 is busy.
Each die serves operations in order, one at a time, with no preemption.
Read throughput is completed read bytes divided by virtual time from the
first offered read to the last read completion, including the drain.
Read latency starts at each offered arrival and ends at completion. With
48 reads, nearest-rank p99 is the slowest sample.

This model fixes the offered trace and excludes host and controller timing.
It demonstrates the mechanism. The size of the effect on a particular SSD
depends on that device and workload.
