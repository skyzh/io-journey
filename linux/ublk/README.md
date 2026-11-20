# Run ext4 on the Rust block device

This lab exports the same RAM-backed FTL through one Linux ublk queue.
The application writes the same length- and CRC-framed records through an
ext4 mount. Follow its sync to the device's flush and recover the records
from that checkpoint.

From the repository checkout on a Mac with the OrbStack machine `ubuntu`,
run the lab in a private mount namespace.

```sh
orb -m ubuntu -u root unshare --mount --propagation private bash linux/ublk/smoke.sh
```

The command was tested on Ubuntu 26.04.1 arm64, kernel
`7.2.12-orbstack-00394-gbecdd6130068`, with Rust 1.93.1. The guest needs
`/dev/ublk-control`, ext4 tools, and `/dev/shm` and `/run` on tmpfs. Cargo
fetches the locked Linux adapter dependencies into `/dev/shm`. The script
uses the guest's libclang or an explicit `LIBCLANG_PATH`. On the tested guest
it extracts Ubuntu's matching libclang package into the temporary directory.
The userspace demo and learner checkpoints still build offline from the
root crate.

The script checks the allocated device's ID, server PID, geometry, and cache
attributes before formatting it. The 32 MiB device presents 4 KiB logical
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
5. The script removes the lab device and confirms the mount and device nodes
   are gone.

The simulated cut replaces the Rust block device's live state while the
ublk server stays running. It uses the checkpoint taken after the
application's sync even when unmount sends more flushes. The guest keeps
the flash images in process memory and all build files, logs, and adapter
metadata on tmpfs.
