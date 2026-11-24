![I/O Journey: Build a Storage Stack in Rust. A record travels down through the application, filesystem, block queue, FTL and NAND flash.](./course/book/io-journey-banner.svg)

# I/O Journey

[![Built with Raft](https://raw.githubusercontent.com/raft-builders/awesome-raft-build/main/badge/built-with-raft.svg)](https://raft.build)

I/O Journey is a hands-on Rust course about what happens when an application
writes a file. Build a small storage stack, connect it to Linux, and see which
writes survive a simulated power cut.

**[Start the course](course/START.md)** · **[Browse the chapters](course/README.md)**

## 🚧 Work in Progress

Chapters, starter code, and commands may change. This course was written with
LLMs and has yet to receive Chi's full review.

## What you’ll build

```mermaid
flowchart TD
    App["Application I/O"] --> FS["Course filesystem"]
    App --> Linux["Linux labs · ext4 and FUSE"]
    FS --> Queue["Block queue"]
    Linux --> Queue
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
