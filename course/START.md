# Start here

You will implement a storage stack in Rust and follow an acknowledged
append down to NAND. Start by running the complete stack. In the first
chapter you will replace two FTL operations, check their recovery rules,
and run that code on an ext4 mount.

## Set up Linux

Use a Linux checkout with Bash, util-linux, and a Rust toolchain with
Cargo. On a Mac with an OrbStack machine named `ubuntu`, open a guest shell
from the course checkout with `orb -m ubuntu`. Run the following from the
checkout inside Linux.

```sh
rustc --version
cargo --version
findmnt -n -o FSTYPE,OPTIONS -T /dev/shm
ls course/learner.sh course/learner/ftl.rs
```

The filesystem command should report `tmpfs` without `noexec`. Cargo
uses it for temporary and build files and executes the compiled checks
there. The demo and first chapter checks run as an ordinary user with
no third-party Rust dependencies. The course was checked on OrbStack Ubuntu
26.04.1 arm64 with Rust 1.93.1. The first ext4 lab lists the extra kernel
tools and privileges it needs.

## Run the reference and begin your implementation

Run the [append-and-recover demo](../README.md#run-the-demo). Inside the
guest, use the command beginning with `bash -eu -c`. From the Mac checkout
shell, use the full `orb -m ubuntu` command. Find the two acknowledged
records, the visible unflushed record, and the two records that survive
the cuts.

Keep editing in this checkout as you work through the [chapters](README.md).
Start with [NAND to SSD](01-ftl.md), where the first check exercises supplied
NAND programming and erasure before you add the FTL operations.
