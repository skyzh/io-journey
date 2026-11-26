# Set up your environment

You can run the reference demo on macOS or Linux with Rust and Cargo.
The full learner test sequence uses a Linux shell and later compiles an
adapter for Linux's `io_uring` interface. The mounted labs also use
Linux kernel features to expose and mount the storage stack you build.

## Set up Linux

Use a Linux machine, a remote Linux server, or a Linux VM. On macOS,
use any VM that supports the kernel features needed by the labs you
want to run. OrbStack is one environment the author tested.

Open a Linux shell. You’ll need Git, Rust, Cargo, and Bash.

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

Start with [Chapter 1’s NAND check](01-ftl.md#implement-the-nand-operations).
The check shows the first missing operation in the starter.
Each chapter gives you a command to rerun after each edit.

Keep editing in this one checkout. The checks select learner code
under `course/learner/`, and later chapters build on the modules
you completed earlier. In the final chapter, run `cargo xtask test`
to check the whole stack together. The demo and
`cargo xtask test reference` use the supplied implementation.

Next, start [NAND to SSD](01-ftl.md).
