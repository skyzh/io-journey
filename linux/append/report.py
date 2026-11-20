from pathlib import Path
import sys


def row(path):
    fs, flush, engine, group, depth, warm, records, byte_count, elapsed, rate, p50, p99 = (
        path.read_text().strip().split("\t")
    )
    expected = path.parent.name
    fields = expected.split("-")
    if (fs, group, engine) != (fields[0], fields[1], fields[3]):
        raise ValueError(f"mismatched case identity in {path}")
    if (depth, warm, records, byte_count) != ("4", "64", "256", "131072"):
        raise ValueError(f"mismatched workload in {path}")
    if int(flush) < 1 or int(elapsed) < 1 or not 0 < int(p50) <= int(p99) <= int(elapsed):
        raise ValueError(f"inconsistent durability or elapsed time in {path}")
    return (
        f"| {fs} | {group} | {fields[2]} | {engine} | {float(rate):.1f} | "
        f"{int(p50) / 1000:.1f} | {int(p99) / 1000:.1f} | {flush} |"
    )


capture = Path(sys.argv[1])
cases = [
    capture / f"{fs}-{group}-{index}-{engine}" / "result.tsv"
    for fs in ("ext4", "fuse")
    for group in (4, 32)
    for index, engine in enumerate(("sync", "uring", "uring", "sync"), 1)
]
rows = [row(path) for path in cases]
print("| Filesystem | Group | Run | Engine | Records/s | Ack p50 µs | Ack p99 µs | Saved FTL flush |")
print("| --- | ---: | ---: | --- | ---: | ---: | ---: | ---: |")
print("\n".join(rows))
