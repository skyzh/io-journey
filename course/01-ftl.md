# 1. NAND to SSD

A logical block address (LBA) stays the same when its bytes move. Our NAND
model programs erased pages and erases whole blocks. The flash
translation layer (FTL) bridges the two by mapping each LBA to the page
that currently holds its data. An overwrite programs a new page. The
old contents remain on NAND until collection can reclaim their block.

Work in the checkout from [Start here](START.md). Edit
[`course/learner/ftl.rs`](learner/ftl.rs) through both steps. The constructor,
page allocation, victim selection, flush, and fault injection are supplied.

## Publish a programmed page

Run the predecessor check and then the mapping check.

```sh
./course/learner.sh previous
./course/learner.sh map
```

The first command checks page programming and whole-block erasure. The
second starts red because the first programmed page has no mapping. The
failure points to the test in `src/ftl.rs`. Make your edit in the linked
learner starter.
Implement `publish_mapping` so the LBA points to the newly programmed page
and the method returns its earlier location, if any. The caller has
already checked the LBA and programmed the new page successfully.
Use `self.map.replace(lba, page)` to record the change.

Run `./course/learner.sh map` again. An overwrite should change the mapping
and expose the new bytes while the old physical page stays programmed.
This is the live map. A flush saves the pages and map together for reopen.

## Relocate live pages before erasing

Run `./course/learner.sh collect`. The fixture overwrites distinct blocks
until collection must move a live page. The starter reports that live-page
relocation needs an implementation.

In `relocate_live_pages`, visit mappings that point into `victim_pages`.
For each one, choose an erased destination outside the victim, copy the
source bytes into it, and update the mapping after the program succeeds.
Victim selection has already checked that enough destinations exist. The
caller erases the victim only after your method succeeds. A stale page has
no LBA pointing to it and can be reclaimed with the rest of the block.
Use the supplied [`Nand`](../src/nand.rs) methods on `self.nand`.
`erased_pages` lists destinations, `read` gives the source, and `program`
takes an owned `Block`. Clone the source before programming the destination.
Use `self.map.replace(lba, destination)` to record a relocated page.
Keep the `_trace` parameter in the function signature. You can leave it
unused or use it to record relocations for debugging.

```sh
./course/learner.sh collect
./course/learner.sh check
```

The first check verifies that a live page moved and that all eight blocks
reopen with the expected bytes. The full check also tests exact torn bytes,
the case with no page erased in both images, and repeated tears followed by
another durable write. A torn program leaves the last flushed mapping
pointing to the previous complete page.

## Use your FTL from ext4

After the offline checks pass, run the ext4 lab from this checkout. It uses
a disposable guest ublk device. Check the [ublk prerequisites](../linux/ublk/README.md)
for the kernel device, ext4 tools, libclang, and temporary Cargo dependencies.
The build and mount run as guest root, so Rust and Cargo must be available
to root. The mount uses a private namespace. Both `/dev/shm` and `/run` must
be on tmpfs. From the Mac checkout shell, run the full OrbStack command.

```sh
orb -m ubuntu -u root unshare --mount --propagation private bash linux/ublk/smoke.sh learner
```

Inside the guest or on standalone lab Linux, run this command from the
checkout.

```sh
sudo unshare --mount --propagation private bash linux/ublk/smoke.sh learner
```

The `learner` argument builds the adapter with `course/learner/ftl.rs`.
A missing or unbuildable implementation stops the build before a device is
allocated.

Look for the learner mapping trace, then the saved checkpoint from the app's
sync. The script reads the unsynced third record, unmounts, and reopens the
saved image. The read-only remount must recover `first` and `second` with
their checksums. The small ext4 workload checks placement and recovery.
The offline collection fixture forces live-page relocation separately.
