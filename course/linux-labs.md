# Linux labs

The main course runs on macOS or Linux and doesn't need any of these labs.
Each one takes code you wrote in a main chapter and runs it on a real
Linux kernel, where ordinary programs and the kernel's own block layer can
use it. Do a lab after the chapter it builds on.

They need a Linux machine or VM and a root shell. See
[mounted labs](START.md#mounted-labs) in Setup.

| Lab | Builds on | What the real system adds |
| --- | --- | --- |
| [Mount your filesystem](linux-filesystem.md) | [3. Blocks to files](03-filesystem.md) | Your journal behind a FUSE mount, on a disk image and then on your FTL and queue through ublk |
| [Append on a real kernel](linux-append.md) | [4. Efficient application I/O](04-append.md) | The append engine on Linux `io_uring`, compared on ext4 and your FUSE filesystem, with a power cut and recovery |

More labs are planned. See [planned labs](planned-labs.md).
