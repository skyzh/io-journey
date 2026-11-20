# 3. Blocks to files

Keep your FTL and queue from [Blocks in flight](02-queue.md). Run
`./course/learner.sh flush` and `./course/learner.sh dispatch` first.
In the [reference demo](../README.md#run-the-demo), follow the append's
data writes and directory update. The [fixed-extent filesystem](../src/filesystem.rs)
records a length after queuing its data. Reopening uses that length
to decide which bytes belong to the file.

The filesystem in this chapter has names, inodes, and allocatable
blocks. Its journal makes a new directory version recoverable. File
sync updates an inode under its already durable name. Directory sync
publishes changes to names, such as a rename over another file.

## Make the redo commit recoverable

Open [`course/learner/journal.rs`](learner/journal.rs) and run
`./course/learner.sh commit`. The error points to the first transaction.
Implement `persist_redo` using the supplied `Disk` interface.

1. Flush the file data already written to the disk.
2. Write the new metadata frame to `REDO` and flush.
3. Write its marker to `COMMIT` and flush.

The marker binds a generation and checksum to that redo frame. The
supplied code then writes and flushes the home block, and clears and
flushes the marker. A failed transaction barrier leaves the filesystem
requiring reopen. Run the commit check again and follow the names that survive
file sync and directory sync.

## Choose what to reopen

Run `./course/learner.sh replay`. A cut after the marker is durable
reaches the missing committed-redo branch of `read_committed`.
The starter handles an empty marker by reading the home block.
For a valid marker, read `REDO`, compare its `frame_checksum` with
the marker, and decode the metadata against `disk.block_count()`.
Compare the decoded generation too. Report a mismatch with the
supplied `corrupt` helper. Return the validated metadata and
`Some(redo)` so the supplied replay step can reinstall home and
clear the marker.

Run the replay check again. A cut before the marker recovers the
old name and bytes. A cut at or after the marker recovers the new
name and bytes, with exactly the corresponding allocated blocks.
A cut during replay can leave the marker present. Replaying it
again must reach the same generation. The tests also reject a
damaged frame or a different valid frame at the same generation.

Try the full-device fixture in the same check. If the new end falls
inside an allocated block, the shrink needs a new block to zero the
suffix while preserving the earlier committed bytes. On a full device it fails before
changing the file. An aligned shrink only drops a whole block
reference. After sync it reclaims that block and recovers the
shorter file.

## Mount this filesystem

Use the [FUSE prerequisites](../linux/fuse/README.md) and the same
private guest mount namespace as the earlier labs. Run from the
Mac checkout shell. Inside Linux, run the part beginning with
`unshare` as guest root.

```sh
orb -m ubuntu -u root unshare --mount --propagation private bash linux/fuse/smoke.sh image learner
orb -m ubuntu -u root unshare --mount --propagation private bash linux/fuse/smoke.sh ublk learner
```

The first command uses the learner journal on a tmpfs image. The
second uses the same journal with the learner FTL and queue behind
ublk. Follow the acknowledged append and then the replacement.
The open descriptor still sees the earlier file through the
rename. The recovered name points to the complete replacement
and the final inspection finds two allocated data blocks.
