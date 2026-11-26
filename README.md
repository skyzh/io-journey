![I/O Journey: Build a Storage Stack in Rust. A record travels down through the application, filesystem, block queue, FTL and NAND flash.](./course/book/io-journey-banner.svg)

# I/O Journey

[![Built with Raft](https://raw.githubusercontent.com/raft-builders/awesome-raft-build/main/badge/built-with-raft.svg)](https://raft.build)

I/O Journey is a hands-on Rust course about what happens when an application
writes a file. Build a small storage stack on macOS or Linux and see which
writes survive a simulated power cut. Optional labs connect it to a Linux kernel.

**[Start the course](https://skyzh.github.io/io-journey/00-architecture.html)** · **[Browse the chapters](https://skyzh.github.io/io-journey/00-architecture.html#what-you-build)**

## 🚧 Work in Progress

Chapters, starter code, and commands may change. This course was written with
LLMs and has yet to receive Chi's full review.

## What you’ll build

![In the portable capstone, appends pass through a mock I/O ring and join random I/O at the userspace write-back cache. The cache sits above the journaled filesystem and host scheduler. The device queue, flash translation layer, and simulated NAND are also used in the optional Linux labs through ublk.](./course/book/assets/storage-stack.svg)

- [NAND to SSD](course/01-ftl.md) — Implement NAND page operations, then make flash behave like a disk
  and test overwrite, collection, and recovery.
- [Blocks in flight](course/02-queue.md) — Build a block queue and see how it
  handles competing requests.
  [Host block scheduling](course/02a-scheduler.md) compares pass-through
  and bounded read-first dispatch ahead of the device queue.
- [Blocks to files](course/03-filesystem.md) — Build a userspace write cache
  and a journaled filesystem.
- [Efficient application I/O](course/04-append.md) — Build an append log, check
  completions on a mock I/O ring, and run the whole stack.

Sponsored by [Raft.build](https://skyzh.github.io/io-journey/sponsor.html).

## License

The source code is licensed under [Apache 2.0](LICENSE).
The book text is © 2026 Alex Chi Z and is licensed under
[CC BY-NC-SA 4.0](https://creativecommons.org/licenses/by-nc-sa/4.0/).
