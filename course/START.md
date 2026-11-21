# Start here

You will implement a storage stack in Rust and follow an acknowledged
append down to NAND. Start by running the complete stack. In the first
chapter you will replace two FTL operations, check their recovery rules,
and run that code on an ext4 mount.

## Set up Linux

Use a Linux checkout with a Rust toolchain and Cargo. The lab helpers need
Bash and util-linux. On a Mac with an OrbStack machine named `ubuntu`, open
a guest shell from the course checkout with `orb -m ubuntu`.

From the checkout inside Linux, check the toolchain and RAM filesystem.

```sh
rustc --version
cargo --version
findmnt -n -o FSTYPE,OPTIONS -T /dev/shm
```

The filesystem command should report `tmpfs` without `noexec`. Set these
paths once in each guest shell so Cargo also builds the command runner in
RAM.

```sh
export CARGO_HOME="/dev/shm/io-journey-cargo-$USER"
export CARGO_TARGET_DIR="/dev/shm/io-journey-target-$USER"
export TMPDIR=/dev/shm
```

Run the demo and offline checks as an ordinary user. The demo and first
chapter checks have no third-party Rust dependencies. The course was
checked on OrbStack Ubuntu 26.04.1 arm64 with Rust 1.93.1.

### Mounted labs

Run these from the checkout in a root Linux shell. On a Mac, open it with
`orb -m ubuntu -u root` from the checkout. Repeat the tmpfs setup above
in that shell. Each mounted Cargo command uses a private mount namespace.
The lab pages list their kernel and tool prerequisites.

## Run the reference and begin your implementation

```sh
cargo xtask demo
```

Find the two acknowledged records, the visible unflushed record, and
the two records that survive the cuts. Use `cargo xtask --help` to see
the available commands.

Keep editing in this checkout as you work through the [chapters](README.md).
Start with [NAND to SSD](01-ftl.md), where the first check exercises supplied
NAND programming and erasure before you add the FTL operations.
