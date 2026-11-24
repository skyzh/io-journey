![I/O Journey: Build a Storage Stack in Rust. A record travels down through the application, filesystem, block queue, FTL and NAND flash.](./course/book/io-journey-banner.svg)

# I/O Journey

[![Built with Raft](https://raw.githubusercontent.com/raft-builders/awesome-raft-build/main/badge/built-with-raft.svg)](https://raft.build)

I/O Journey is a hands-on Rust course about what happens when an application
writes a file. Build a small storage stack, connect it to Linux, and see which
writes survive a simulated power cut.

**[Start the course](https://skyzh.github.io/io-journey/START.html)** · **[Browse the chapters](https://skyzh.github.io/io-journey/00-architecture.html#what-you-build)**

## 🚧 Work in Progress

Chapters, starter code, and commands may change. This course was written with
LLMs and has yet to receive Chi's full review.

## What you’ll build

The Linux append lab runs both I/O engines on both filesystems through ublk.
The diagram also shows where physical storage and the planned NVMe lab fit.

```mermaid
flowchart TD
    App["Append log"]
    App --> Sync["Synchronous engine"]
    App --> Uring["io_uring engine"]
    Sync --> Ext4["ext4"]
    Sync --> Fuse["Course filesystem via FUSE"]
    Uring --> Ext4
    Uring --> Fuse
    Ext4 --> Block["Linux block layer"]
    Fuse --> Block
    Block --> Ublk["ublk virtual block device · current lab"]
    Block --> Hardware["Physical block device"]
    Block -.-> NvmeHost["Linux NVMe driver · TBD"]
    NvmeHost -. NVMe commands .-> NvmeController["Emulated NVMe controller · TBD"]
    NvmeController -.-> Queue["Rust block request and completion queue"]
    Ublk --> Queue
    Queue --> FTL["Flash translation layer (FTL)"]
    FTL --> NAND["Simulated NAND"]
```

- [NAND to SSD](course/01-ftl.md) — Build the layer that makes flash behave
  like a disk, then use it under Linux with ext4.
- [Blocks in flight](course/02-queue.md) — Build a block queue and see how it
  handles competing requests.
- [Blocks to files](course/03-filesystem.md) — Build a small filesystem and
  see what it recovers after a crash.
- [Efficient application I/O](course/04-append.md) — Build an append log and
  compare two ways of writing to Linux filesystems.

Sponsored by [Raft.build](https://skyzh.github.io/io-journey/sponsor.html).
