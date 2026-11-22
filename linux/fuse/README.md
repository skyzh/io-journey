# Recover the learner filesystem through FUSE

Run from the checkout in the [root guest shell](../../course/START.md#mounted-labs)
with Rust, Python 3, `/dev/fuse`, and `/dev/shm` on tmpfs.

```sh
cargo xtask fuse image
cargo xtask fuse ublk
```

On `image`, the script creates an 8 MiB file on tmpfs and saves copies
at the acknowledged sync points. On `ublk`, it verifies the ownership
and geometry of a fresh RAM-backed block device and saves the exact
FTL checkpoint from the last flush. The latter uses the
[ublk lab setup](../ublk/README.md). Both runs build and keep scratch
data in guest tmpfs.

The app appends two length- and CRC-framed records, syncs the file and
root directory, then appends an unsynced third record. After reopen it
checks that only the first two return. Next it syncs a temporary file,
renames it over the log, and syncs the root directory. An open descriptor
to the old file keeps its contents until close. The script cuts after an
unsynced truncate, then checks the replacement record, its two allocated
blocks, and mount and server cleanup.

## Follow an update to disk

The filesystem supports up to 12 root files of up to 512 KiB each with
names up to 48 bytes. It handles reads, writes, append, truncation, unlink,
and root-only rename and replacement. File and directory sync are explicit.
The adapter uses FUSE direct I/O for file handles and aligned direct I/O
for the backend. Metadata timestamps and ownership are fixed for the
mount. [FUSE direct I/O](https://docs.kernel.org/filesystems/fuse/fuse-io.html)

| Blocks | Contents |
| --- | --- |
| 0 | Checksummed home directory and inodes |
| 1 | Checksummed redo copy of the next metadata |
| 2 | Commit marker with the redo generation and checksum |
| 3 onward | File data allocated in 4 KiB blocks |

Writes copy changed blocks to unused locations. Allocation keeps blocks
referenced by the durable directory, the live directory, and open
unlinked files. This preserves the prior version until the transaction
commits. File sync flushes data and updates the inode under its already
durable name. The root directory sync publishes new names and renames
along with the current file contents. [Linux file and directory sync](https://man7.org/linux/man-pages/man2/fsync.2.html)

A metadata update flushes data, writes and flushes redo, then writes and
flushes the commit marker. It installs and flushes the home copy before
clearing and flushing the marker. Reopen loads the home copy when the
marker is empty, or validates and replays the committed redo. An invalid
committed frame returns a corruption error.

## Run the crash checks

From the same checkout, run the focused model checks without a mount.
The command compiles offline and keeps the target in guest tmpfs.

```sh
cargo xtask test reference journal_fs
```

The tests reopen the RAM-backed FTL before the transaction and after
each of its five flushes. They check names, exact contents and allocation.
The old version survives through the redo flush. The replacement survives
from the commit-marker flush onward, including a second reopen after replay.
