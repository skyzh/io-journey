# Set up your environment

Run the reference demo and the focused checks for Chapters 1–3 on macOS
or Linux. Use Linux for the mounted labs and the Chapter 4 engine
checks that build the `io_uring` adapter.

## Get the course

Open a terminal with Git, Rust, Cargo, and Bash.

```sh
git --version
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

Run the demo and focused checks as an ordinary user. Cargo uses its
normal cache and build directories on disk. These checks use the
standard library and run offline.

## Run the Linux labs

Use a Linux machine, a remote Linux server, or a Linux VM with the
kernel features each lab needs. OrbStack is one VM environment tested
for the mounted labs. Open the course checkout in your Linux shell.

### Mounted labs

Run the mounted commands from the checkout in a root Linux shell.
The runners use private mount namespaces and fresh, disk-backed
Cargo caches. Give the build access to the Cargo registry or a
configured mirror, starting with the mounted lab in Chapter 2. The
lab instructions list the required kernel devices and tools. The queue
and append benchmarks keep their captures in `/dev/shm`.

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
you completed earlier. The demo and `cargo xtask test reference`
use the supplied implementation.

In Chapter 4, `cargo xtask learner receipt` also runs offline on
either platform. Run the engine checks, `cargo xtask learner completions`
and `cargo xtask learner ack`, in an ordinary Linux shell. Use that shell
for the full `cargo xtask test` as well. Those commands build the Linux
adapter and need registry or mirror access if its locked dependencies
are missing from the cache.

Next, start [NAND to SSD](01-ftl.md).
