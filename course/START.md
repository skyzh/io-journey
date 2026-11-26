# Set up your environment

Run the reference demo and the focused learner checks on macOS or
Linux. The mounted labs use the real Linux block and I/O interfaces
and need a Linux guest or machine.

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
configured mirror, starting with the mounted lab in Chapter 2. If you
use a mirror, add its source settings to the checkout's
`.cargo/config.toml` so the mounted runners can read them.
The lab instructions list the required kernel devices and tools.
The queue and append benchmarks keep their captures in `/dev/shm`.

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
you completed earlier. `cargo xtask demo` and
`cargo xtask test reference` use the supplied implementation.

In Chapter 4, the focused receipt, mock completion, and acknowledgment
checks run offline on either platform. Run the completed stack after
Chapter 4.

```sh
cargo xtask demo learner
```

The app checks aligned reads, writes, and mixed I/O. It sees three log
records before a simulated cut and recovers the two it acknowledged
after sync. `cargo xtask test` runs the full sequence of focused checks
and finishes with the same recovery run.
