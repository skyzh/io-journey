# I/O Journey

I/O Journey follows storage requests across software and hardware boundaries with small, observable experiments. The private pilot pairs one overview—[From `read(2)` to NAND](docs/01-from-read-2-to-nand.md)—with a bounded Rust Network Block Device (NBD) server.

The lab makes four block operations visible without attaching a kernel device:

- `READ` and `WRITE` move bytes at an offset;
- `FLUSH` synchronizes the lab's regular backing file before success;
- `TRIM` zeroes a range as this server's deterministic teaching policy.

TRIM does **not** generally promise zero-filled reads. The lab records a software protocol path; it does not measure an SSD, NVMe controller, FTL, or NAND.

## Safety boundary

The lab is deliberately unprivileged. It uses one new mode-0700 temporary directory, a private Unix socket, an owner-only listener-readiness marker, and a newly created 1 MiB regular file. It never uses `/dev/nbd*`, `sudo`, `modprobe`, `mkfs`, a mount, a partition table, or a real disk path. Existing readiness, socket, image, log, and output paths are rejected.

## Local protocol check

Use a current stable Rust toolchain. The crate has no third-party dependencies.

```console
cargo build --release

./target/release/rust-nbd-lab verify /tmp/io-journey-check-1
./target/release/rust-nbd-lab verify /tmp/io-journey-check-2

cmp /tmp/io-journey-check-1/events.jsonl \
    /tmp/io-journey-check-2/events.jsonl
```

Each fresh run negotiates a 1 MiB export, then performs write → flush → read → trim → flush → read → disconnect. The two event logs must be byte-identical. The check also verifies write visibility, this lab's trim-zeroing policy, the backing-file bound, and socket cleanup.

The two explicit verification directories contain disposable evidence. Remove only those exact paths after inspecting them.

## Independent `fio` check on Linux

The supplied experiments need a Linux `fio` build with its libnbd-backed `nbd` engine and Python 3's standard JSON decoder. Confirm both surfaces before running anything:

```console
command -v python3
fio --enghelp=nbd
```

Then run each finite experiment into a new output directory:

```console
./run-fio.sh ./target/release/rust-nbd-lab read  evidence/read-a
./run-fio.sh ./target/release/rust-nbd-lab write evidence/write-a
./run-fio.sh ./target/release/rust-nbd-lab flush evidence/flush-a
./run-fio.sh ./target/release/rust-nbd-lab trim  evidence/trim-a
```

For reproducibility, run all four jobs twice and compare each pair of `events.jsonl` files. Preserve the corresponding `fio.json` files: latency is environmental, while operation type, offset, length, and order are the deterministic contract. Each output directory also keeps fio's raw standard output in `fio.stdout.log`, separate from the parseable JSON artifact, so libnbd connection diagnostics remain available.

The current macOS development host has no suitable `fio`/libnbd surface. The local Rust client is protocol evidence, not a substitute for that independent Linux acceptance run. Do not start a VM merely to bypass this gate on an unattended machine.

## Pilot map

- [From `read(2)` to NAND](docs/01-from-read-2-to-nand.md): the conditional request path and the boundaries the lab can observe.
- `src/main.rs`: the small CLI dispatcher.
- `src/server.rs`: root, listener, connection, cleanup, and verification lifecycle.
- `src/protocol.rs`: fixed-newstyle negotiation, framing, and the local protocol probe.
- `src/export.rs`: bounded storage operations and deterministic event recording.
- `fio/*.fio`: four finite, 4 KiB-aligned experiments.
- `run-fio.sh`: the validated private-runtime runner.

Later units may cover queueing and `io_uring`, workload-relative steady state, latency injection, FTL garbage collection, virtualization, and links to Mini-LSM. They are not part of this pilot.
