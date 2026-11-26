# Linux lab: mount your filesystem

This side chapter runs the journaled filesystem from
[Blocks to files](03-filesystem.md) under Linux, so ordinary programs can
use it. It needs a Linux machine or VM and a root shell. The main course
doesn't depend on it.

## Run the lab

Use the [FUSE prerequisites](https://github.com/skyzh/io-journey/blob/main/linux/fuse/README.md).
From the checkout in the [root guest shell](START.md#mounted-labs), run
both backends.

```sh
cargo xtask fuse image learner
cargo xtask fuse ublk learner
```

The first command uses the learner journal on a disk-backed image. The
second uses the same journal with the learner FTL and queue behind
ublk. The lab supplies the application workload. Follow its acknowledged
append and then the replacement.
The open descriptor still sees the earlier file through the
rename. The recovered name points to the complete replacement
and the final inspection finds two allocated data blocks.
