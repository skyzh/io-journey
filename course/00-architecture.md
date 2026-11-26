# Overview: one write through the stack

When an application appends to a file and calls sync, its bytes pass
through several layers before they rest on flash. In this course you build
that stack in Rust, starting with a simulated NAND chip:

<ol class="write-path" aria-label="One append through the storage stack">
  <li><strong>Application</strong><span>Write the record bytes at the end of the log, then sync the file</span></li>
  <li><strong>Filesystem</strong><span>Translate the byte offset into a logical block and update the file length</span></li>
  <li><strong>Block queue</strong><span>Send 4 KiB block requests, then flush the earlier writes</span></li>
  <li><strong>Flash translation layer</strong><span>Map each updated block to a fresh page</span></li>
  <li><strong>Simulated NAND</strong><span>Program pages and erase whole blocks</span></li>
</ol>

Each layer talks to the one below it in a smaller vocabulary. The
application knows files and bytes. The filesystem turns them into numbered
4 KiB logical blocks, each identified by a logical block address (LBA). The
queue decides which block request runs next. The flash translation layer
(FTL) decides which NAND page holds each LBA. The NAND model stores pages
and enforces the rules real flash has.

The rest of this page follows one append through those layers. You don't
need to understand every step yet. Each chapter returns to its own layer
in detail.

## Follow one append

The [reference demo](START.md#run-the-reference-demo) appends two records
to `/events.log` and syncs the file. The filesystem keeps the file length
in logical block 0 (LBA 0) and the record bytes in LBA 1. Watch where
those two blocks move as the second record is appended.

### 1. Read the block holding the first record

The first record takes 13 bytes (an 8-byte header and the 5-byte payload
`first`). By the time request #4 reads LBA 1, the queue has completed the
first record’s data and length writes. Here is the live mapping then.

<figure class="block-map">
  <figcaption>Mapping when the second append reads LBA 1</figcaption>
  <ul>
    <li><div><strong>LBA 0</strong><span>File length: 13 bytes</span></div><div><strong>NAND page 2</strong><span>Directory block</span></div></li>
    <li><div><strong>LBA 1</strong><span>Record bytes</span></div><div><strong>NAND page 1</strong><span>Holds <code>first</code></span></div></li>
  </ul>
</figure>

### 2. Append at byte offset 13 and write two blocks

The second record takes 14 bytes (the header plus `second`). The
filesystem copies those bytes into the block it read, starting at
offset 13. It queues the updated data block (#5) and the new file
length (#6). The FTL writes both blocks to fresh NAND pages. Pages 1
and 2 become stale as their mappings move.

<figure class="block-map">
  <figcaption>Mapping after writes #5 and #6, saved by flush #7</figcaption>
  <ul>
    <li><div><strong>LBA 0</strong><span>File length: 27 bytes</span></div><div><strong>NAND page 4</strong><span>Replaces page 2</span></div></li>
    <li><div><strong>LBA 1</strong><span>Record bytes</span></div><div><strong>NAND page 3</strong><span>Holds <code>first</code> and <code>second</code></span></div></li>
  </ul>
</figure>

### 3. Sync the file, then recover the saved mapping

The application syncs the file. The queue runs its flush (#7) after
both block writes complete. The FTL saves the two new mappings and
their page contents in the simulated durable image. Once sync returns,
the application acknowledges both records.

The demo then appends `volatile` without syncing again. The two blocks
move to fresh pages again, and a read before the cut sees all three
records. Then the demo cuts power. Nothing after flush #7 was saved, so
the FTL reopens from the saved mapping:

<figure class="block-map">
  <figcaption>Live mapping just before the cut</figcaption>
  <ul>
    <li><div><strong>LBA 0</strong><span>File length: 43 bytes</span></div><div><strong>NAND page 6</strong><span>Not flushed</span></div></li>
    <li><div><strong>LBA 1</strong><span>Record bytes</span></div><div><strong>NAND page 5</strong><span>Holds <code>first</code>, <code>second</code> and <code>volatile</code></span></div></li>
  </ul>
</figure>

<figure class="block-map">
  <figcaption>Mapping after reopening from flush #7</figcaption>
  <ul>
    <li><div><strong>LBA 0</strong><span>File length: 27 bytes</span></div><div><strong>NAND page 4</strong><span>Saved by flush #7</span></div></li>
    <li><div><strong>LBA 1</strong><span>Record bytes</span></div><div><strong>NAND page 3</strong><span>Holds <code>first</code> and <code>second</code></span></div></li>
  </ul>
</figure>

The recovered file has length 27 and contains only the two acknowledged
records. Last, the demo cuts power partway through programming a page
and checks that the same two records survive that too.

## What you build

| Chapter | Layer | You add |
| --- | --- | --- |
| [1. NAND to SSD](01-ftl.md) | NAND and FTL | Read, program, and erase NAND, then publish and relocate logical blocks |
| [2. Blocks in flight](02-queue.md) | Block queue | Bounded requests, flush ordering, and a [host scheduler](02a-scheduler.md) that chooses which request to send next |
| [3. Blocks to files](03-filesystem.md) | Filesystem | A userspace write cache and a journal that makes file and directory changes recoverable |
| [4. Efficient application I/O](04-append.md) | Application | Record recovery, completion checks on a simulated I/O ring, and acknowledgement after sync |

Chapter 1 starts with the NAND operations and builds the FTL on top.
Each later chapter runs on the code you wrote in the earlier ones. After
Chapter 4, one command runs your whole stack, simulates a power cut and
checks what survived. Everything in the main chapters runs on macOS or
Linux. The optional [Linux labs](linux-labs.md) run your learner modules
with real kernel interfaces.

Next, [set up your environment and run the demo](START.md).
