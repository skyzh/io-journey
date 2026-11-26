# Run ext4 on the Rust block device

This lab exports the same RAM-backed FTL through one Linux ublk queue.
The application writes the same length- and CRC-framed records through an
ext4 mount. Follow its sync to the device's flush and recover the records
from that checkpoint.

Run from the checkout in the [root guest shell](../../course/START.md#mounted-labs).

```sh
cargo xtask ext4
```

The command was tested on Ubuntu 26.04.1 arm64, kernel
`7.2.12-orbstack-00394-gbecdd6130068`, with Rust 1.93.1. The guest needs
`/dev/ublk-control`, ext4 tools, and `/run` on tmpfs for ublk metadata.
Cargo builds the adapter under `/var/tmp`. The script
uses the guest's libclang or an explicit `LIBCLANG_PATH`. On the tested guest
it extracts Ubuntu's matching libclang package into the temporary directory.
The userspace demo and learner checkpoints still build offline from the
root crate.

The script checks the allocated device's ID, character-node identity, server
PID, and run marker before formatting it. It also checks the geometry and
cache attributes. The 32 MiB device presents 4 KiB logical
blocks and a write-back cache with `fua=0`. Linux sends separate flush requests.
Each successful flush publishes the FTL image in memory. [Linux ublk](https://docs.kernel.org/block/ublk.html) · [Linux cache flush and FUA](https://docs.kernel.org/block/writeback_cache_control.html)

Look for these events in the output.

1. The guest creates a ublk device and mounts ext4.
2. The app appends `first` and `second`, syncs the file and its directory,
   and acknowledges both records.
3. The server shows a new FTL flush and retains that exact RAM checkpoint
   before unmount. A later append is readable at this point.
4. The script unmounts ext4, invalidates the kernel block cache, and reopens
   the captured checkpoint. The read-only remount reads the reopened device
   and verifies only `first` and `second` with their checksums.
5. The script reaps the server and verifies that the mount, both device
   nodes, and runtime metadata are gone. If the socket stop fails, it
   checks the original allocation and run marker before deleting an
   orphaned device after the server exits.

The simulated cut replaces the Rust block device's live state while the
ublk server stays running. It uses the checkpoint taken after the
application's sync even when unmount sends more flushes. The guest keeps
the flash images in process memory, build files and logs under
`/var/tmp`, and adapter metadata under `/run`.

To run this optional smoke check with the FTL you wrote in the course,
pass `learner` after completing the FTL tests.

```sh
cargo xtask ext4 learner
```

The adapter builds with `course/learner/ftl.rs` before allocating a
device. Look for the learner mapping trace before the checkpoint and
recovery output.

Use the [queue benchmark](QUEUE_BENCHMARK.md) to measure request size,
depth, and a scheduling change on the same RAM-backed device.
