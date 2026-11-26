# Set up your environment

You can run the reference demo on macOS or Linux with Rust and Cargo.
The full learner test sequence uses a Linux shell and later compiles an
adapter for Linux's `io_uring` interface. The mounted labs also use
Linux kernel features to expose and mount the storage stack you build.

## Set up Linux

Use a Linux machine, a remote Linux server, or a Linux VM. On macOS,
use any VM that supports the kernel features needed by the labs you
want to run. OrbStack is one environment the author tested.

Open a Linux shell and check that Rust, Cargo, and Bash are available.

```sh
rustc --version
cargo --version
bash --version
```

Clone the course and enter the checkout. If you already cloned it, open
that checkout instead.

```sh
git clone https://github.com/skyzh/io-journey.git
cd io-journey
```

Run the demo and learner tests as an ordinary user. Cargo uses its
normal cache and build directories. The append chapter needs Cargo
registry access to fetch its locked Linux dependencies. The earlier
chapters use the standard library.

### Mounted labs

When a chapter calls for a mounted lab, open a root shell on Linux
and run the command from the checkout. Each mounted command uses a
private mount namespace. The lab instructions list the required kernel
devices and tools. The queue and append benchmarks keep their captures
in `/dev/shm` to avoid writing measurement output to disk during a run.

## Run the reference demo

```sh
cargo xtask demo
```

Find the two acknowledged records, the visible unflushed record, and
the two records that survive the cuts.

## Test your own implementation

From the course checkout, use the same command after each edit.

```sh
cargo xtask test
```

The command checks the supplied NAND first, then tests the exercises in
chapter order. It stops at the first failing check and names the step
to work on next. When that step passes, the command can reach the next
exercise, which may still fail until you implement it.

The command sets Cargo features to compile your files under
`course/learner/`. Later features keep your earlier modules active, so
the queue builds on your FTL and the journal and append chapters keep
using your earlier work. Keep editing in this one checkout through the
four chapters. The demo and `cargo xtask test reference` use the supplied
implementation.

Next, start [NAND to SSD](01-ftl.md).
