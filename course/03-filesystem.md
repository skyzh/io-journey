# 3. Blocks to files

Keep your FTL and queue from [Blocks in flight](02-queue.md).

The [reference demo](START.md#run-the-reference-demo) uses a deliberately
small [fixed-extent filesystem](https://github.com/skyzh/io-journey/blob/main/src/filesystem.rs):
one file, whose length lives in LBA 0. Writing the data and then the length
is enough for one append, but not for renaming, replacing or deleting
files. This chapter builds the filesystem the mounted labs use. It has
names, inodes and allocatable blocks, and a journal that makes each new
version of the directory recoverable after a power cut.

## How the journal works

The filesystem keeps all of its metadata in one 4 KiB block: the file names,
each file's inode, length and list of data blocks, and a generation number
that goes up by one with every committed change. The first three LBAs have
fixed jobs:

| LBA | Name | Holds |
| --- | --- | --- |
| 0 | `HOME` | The current committed metadata |
| 1 | `REDO` | The next metadata version, while a change is being committed |
| 2 | `COMMIT` | A marker with that version's generation and a checksum of the `REDO` block, or all zeros when no commit is in progress |
| 3 and up | data | File contents |

File data is copy-on-write: new bytes go to freshly allocated data blocks,
and the committed metadata keeps pointing at the old ones. Nothing changes
for a reader after a power cut until the new metadata is committed. A
commit runs in this order, with a flush after each write. The `Disk`
interface promises only that a successful flush covers all earlier writes,
so the flushes are the only thing that orders these steps.

<ol class="write-path" aria-label="Steps of one journal commit">
  <li><strong>1. Flush the data</strong><span>The new file blocks become durable before any metadata points at them</span></li>
  <li><strong>2. Write <code>REDO</code>, flush</strong><span>The complete new metadata is on the device, but nothing refers to it yet</span></li>
  <li><strong>3. Write <code>COMMIT</code>, flush</strong><span>The commit point: once the marker is durable, the new version wins</span></li>
  <li><strong>4. Install and clear</strong><span>Copy <code>REDO</code> into <code>HOME</code>, flush, zero <code>COMMIT</code>, flush</span></li>
</ol>

You write steps 1–3. The supplied code does step 4. The tests cut power at
the simulator's flush boundaries, where it keeps a durable snapshot of
everything flushed so far. When the filesystem reopens from such a
snapshot, it reads `COMMIT` first and chooses a version:

| Power cut | `COMMIT` on reopen | Recovered version |
| --- | --- | --- |
| Before step 3's flush | All zeros | The old one, from `HOME`. The new data blocks are unreferenced and become free again. |
| After step 3, before step 4 | Valid marker | The new one, from `REDO`, which reopen then installs into `HOME` |
| During step 4, before the cleared marker is flushed | Still valid | The new one again. Installing it twice gives the same result. |
| After step 4's last flush | All zeros | The new one, now in `HOME` |

The marker's checksum and generation make sure reopen never trusts the
wrong `REDO` block. If `REDO` was damaged, or holds a different version
than the marker describes, reopen reports corruption instead of guessing.

`sync_file` commits one file's new data and length under the name it
already has on disk. `sync_dir` commits the whole directory, so it is the
call that makes a create, rename or delete durable.

## Make the redo commit recoverable

Open [`course/learner/journal.rs`](https://github.com/skyzh/io-journey/blob/main/course/learner/journal.rs) and run
`cargo xtask learner commit`. At the first transaction the commit check reports
`implement journal barriers`.
Implement `persist_redo`, steps 1–3 above, using the supplied `Disk` interface
(`write_block` and `flush`).

1. Flush the file data already written to the disk.
2. Write the new metadata frame to `REDO` and flush.
3. Write the marker to `COMMIT` and flush.

`REDO` is defined in the parent module next to `HOME` and `COMMIT`, but the
starter doesn't import it. Add it to the `use super::{...}` line.

If any write or flush fails, return the error. The filesystem then refuses
further updates until it is reopened, because it can't know which of the
writes reached the device. Rerun the check. It follows which names survive
file sync and directory sync.

## Choose what to reopen

Once the commit check passes, run `cargo xtask learner replay`.
The journal suite cuts power after the marker is durable and reaches
the missing committed-redo branch of
`read_committed`.
The starter already handles an all-zeros marker by reading `HOME`. You
handle a valid marker, the middle and last rows of the table above:

1. Read `REDO`.
2. Compute its checksum with `frame_checksum(&redo)` and compare it with the
   checksum in the marker.
3. Decode the metadata with `Metadata::decode(&redo, disk.block_count())`
   and compare its `generation` with the marker's.
4. Report either mismatch with `corrupt("...")`, which returns an
   `InvalidData` error.
5. Return the metadata and `Some(redo)`, so the supplied code can install it
   into `HOME` and clear the marker.

`frame_checksum` and `corrupt` are helpers in the parent module, and the
starter doesn't import them. Add both, along with `REDO`, to the
`use super::{...}` line. Then rename `_generation` and `_checksum` so you
can use them.

Rerun the replay check. A cut before the marker recovers the
old name and bytes. A cut at or after the marker recovers the new
name and bytes, with exactly the corresponding allocated blocks.
A cut during replay can leave the marker present. Replaying it
again must reach the same generation. The tests also reject a
damaged frame, a different valid frame at the same generation, and a
matching checksum with the wrong generation.

Try the full-device fixture in the same check. If the new end falls
inside an allocated block, the shrink needs a new block to zero the
suffix while preserving the earlier committed bytes. On a full device it fails before
changing the file. An aligned shrink only drops a whole block
reference. After sync it reclaims that block and recovers the
shorter file.

## Mount this filesystem

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

Next, [Efficient application I/O](04-append.md) adds the append engine
and compares it on the complete stack.
