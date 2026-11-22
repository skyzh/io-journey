# Start here

You will implement a storage stack in Rust and follow an acknowledged
append down to NAND. Start by running the complete stack. In the first
chapter you will replace two FTL operations, check their recovery rules,
and run that code on an ext4 mount.

## Set up Linux

Use a Linux checkout with a Rust toolchain, Cargo, Bash, `mktemp`, and
`findmnt` from util-linux. On a Mac with an OrbStack machine named `ubuntu`, open
a guest shell from the course checkout with `orb -m ubuntu`.

From the checkout inside Linux, check the toolchain and RAM filesystem.

```sh
rustc --version
cargo --version
findmnt -n -o FSTYPE,OPTIONS -T /dev/shm
```

Once the filesystem command reports `tmpfs` without `noexec`, prepare a
private Cargo directory for this guest shell. This also keeps the command
runner's build in RAM.

```sh
io_journey_cargo=$(mktemp -d /dev/shm/io-journey-cargo.XXXXXX) &&
  export CARGO_HOME="$io_journey_cargo/home" CARGO_TARGET_DIR="$io_journey_cargo/target" TMPDIR="$io_journey_cargo"
```

Run the demo and `cargo xtask test` as an ordinary user. The append
chapter needs Cargo registry access to fetch its locked Linux `io-uring`
dependencies into tmpfs. The earlier chapters use the standard library.
The course was checked on OrbStack Ubuntu 26.04.1 arm64 with Rust 1.93.1.

### Mounted labs

Run these from the checkout in a root Linux shell. On a Mac, open it with
`orb -m ubuntu -u root` from the checkout. Repeat the tmpfs setup above
in that shell. Each mounted Cargo command uses a private mount namespace.
The lab pages list their kernel and tool prerequisites.

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

Next, [follow one write through the stack](00-architecture.md).
