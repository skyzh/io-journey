# Linux labs

The main chapters don't need any of these labs. Their focused checks run
on macOS or Linux, including Chapter 4's mock completion and
acknowledgment stage. Each lab takes code you wrote in a main chapter and runs it on a real
Linux kernel, where ordinary programs and the kernel's own block layer can
use it. Do a lab after the chapter it builds on.

They need a Linux machine or VM and a root shell. See
[mounted labs](START.md#mounted-labs) in Setup.

| Lab | Builds on | What the real system adds |
| --- | --- | --- |
| [Use your queue from Linux](linux-ublk.md) | [2. Blocks in flight](02-queue.md) | Your simulated device under Linux ublk, with ext4 recovery and a mixed random read/write fio comparison |
| [Mount your filesystem](linux-filesystem.md) | [3. Blocks to files](03-filesystem.md) | Your journal behind a FUSE mount, on a disk image and then on your FTL and queue through ublk |
| [Append on a real kernel](linux-append.md) | [4. Efficient application I/O](04-append.md) | The append engine on Linux `io_uring`, compared on ext4 and your FUSE filesystem, with a power cut and recovery |

More labs are planned. See [planned labs](planned-labs.md).
