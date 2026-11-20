import csv
import json
import math
import re
import sys
from collections import defaultdict
from pathlib import Path


EVENT = re.compile(
    r"(?P<time>\d+\.\d+): block_rq_(?P<kind>issue|complete): "
    r"(?P<dev>\d+,\d+) (?P<flags>\S+).*?\) (?P<sector>\d+) \+ "
    r"(?P<sectors>\d+) \S+ \[(?P<end>.*)\]$"
)


def p99(values):
    ordered = sorted(values)
    return ordered[math.ceil(0.99 * len(ordered)) - 1] / 1000 if ordered else None


def observed(value):
    return f"{value:.1f}" if value is not None else "—"


def depth_buckets(job):
    return ", ".join(
        f"{depth}: {share:.1f}%" for depth, share in job["iodepth_level"].items() if share
    )


def fio_job(path, wanted_bytes):
    job, = json.loads(path.read_text())["jobs"]
    actual = job["read"]["io_bytes"] + job["write"]["io_bytes"]
    if job["error"] or actual != wanted_bytes:
        raise ValueError(f"{path.name} reported error {job['error']} or {actual} bytes")
    return job


def event_time(value):
    seconds, fraction = value.split(".", 1)
    return int(seconds) * 10**9 + int(fraction.ljust(9, "0")[:9])


def kernel_events(path, device):
    issues, completions = defaultdict(list), defaultdict(list)
    udev_reads = 0
    for line in path.read_text().splitlines():
        if line.startswith("#") or not line.strip():
            continue
        match = EVENT.search(line)
        if not match or match["dev"] != device:
            raise ValueError(f"unexpected block trace record in {path}")
        key = (match["flags"][0], int(match["sector"]), int(match["sectors"]))
        timestamp = event_time(match["time"])
        if match["kind"] == "issue":
            issues[key].append(timestamp)
            if key[0] == "R" and match["end"].startswith("(udev-"):
                udev_reads += key[2] * 512
        else:
            if int(match["end"]) != 0:
                raise ValueError(f"block completion failed in {path}")
            completions[key].append(timestamp)
    return issues, completions, udev_reads


def checked_trace_stats(path):
    for line in path.read_text().splitlines():
        label, _, value = line.partition(":")
        if label.strip() in ("overrun", "commit overrun", "dropped events") and int(value) != 0:
            raise ValueError(f"trace records were lost in {path}")


def case(path):
    _, depth_text, size_text, device = (path / "case.tsv").read_text().strip().split("\t")
    depth, size = int(depth_text), int(size_text)
    fio_job(path / "warm.json", 2 * 1024**2)
    job = fio_job(path / "fio.json", 16 * 1024**2)
    checked_trace_stats(path / "trace-stats.txt")
    issues, completions, udev_reads = kernel_events(path / "kernel.trace", device)
    with (path / "server.tsv").open(newline="") as stream:
        programs, copies, erases = map(int, stream.readline().lstrip("#").split())
        samples = list(csv.DictReader(stream, delimiter="\t"))
    if not samples or len(samples) != int((path / "profile-rows.txt").read_text()):
        raise ValueError(f"incomplete server capture in {path}")

    server_by_range = defaultdict(list)
    for row in samples:
        if int(row["result"]) < 0:
            raise ValueError(f"server request failed in {path}")
        if row["op"] in ("R", "W"):
            server_by_range[(row["op"], int(row["sector"]), int(row["sectors"]))].append(row)
    data = [row for group in server_by_range.values() for row in group]
    extra_bytes = []
    for operation, fio_direction in (("R", "read"), ("W", "write")):
        server_bytes = sum(int(row["result"]) for row in data if row["op"] == operation)
        issued_bytes = sum(
            key[2] * 512 * len(times) for key, times in issues.items() if key[0] == operation
        )
        fio_bytes = job[fio_direction]["io_bytes"]
        if issued_bytes != server_bytes or server_bytes < fio_bytes:
            raise ValueError(f"inconsistent {operation} bytes across guest, server, and fio in {path}")
        extra_bytes.append(server_bytes - fio_bytes)

    matched = []
    for key, rows in server_by_range.items():
        issued, completed = issues.get(key, []), completions.get(key, [])
        if len(rows) == len(issued) == len(completed) == 1 and completed[0] >= issued[0]:
            matched.append(completed[0] - issued[0])
    return {
        "name": path.name,
        "size": size // 1024,
        "depth": depth,
        "achieved": depth_buckets(job),
        "read_bytes": job["read"]["io_bytes"],
        "write_bytes": job["write"]["io_bytes"],
        "runtime": max(job["read"]["runtime"], job["write"]["runtime"]),
        "bandwidth": (job["read"]["bw_bytes"] + job["write"]["bw_bytes"]) / 1024**2,
        "read_p99": job["read"]["lat_ns"]["percentile"]["99.000000"] / 1000,
        "server_wait_p99": p99(int(row["wait_ns"]) for row in data if row["op"] == "R"),
        "server_service_p99": p99(int(row["service_ns"]) for row in data if row["op"] == "R"),
        "advanced": sum(int(row["advanced"]) for row in data),
        "issues": sum(map(len, issues.values())),
        "completions": sum(map(len, completions.values())),
        "server_requests": len(data),
        "extra_read_bytes": extra_bytes[0],
        "extra_write_bytes": extra_bytes[1],
        "udev_bytes": udev_reads,
        "joined": len(matched),
        "kernel_p99": p99(matched),
        "programs": programs,
        "copies": copies,
        "erases": erases,
    }


def report(rows):
    print("| Case | KiB | Requested depth | fio depth buckets | fio read / write bytes | ms | MiB/s | fio read p99 µs |")
    print("| --- | ---: | ---: | --- | ---: | ---: | ---: | ---: |")
    for row in rows:
        print(
            f"| {row['name']} | {row['size']} | {row['depth']} | {row['achieved']} | "
            f"{row['read_bytes']} / {row['write_bytes']} | {row['runtime']} | "
            f"{row['bandwidth']:.1f} | {row['read_p99']:.1f} |"
        )
    print()
    print("| Case | Server read wait / service p99 µs | Reads advanced | Guest issues / completions | Unique joins / server requests | Guest joined p99 µs | Extra read / write bytes (udev read bytes) | NAND programs / copies / erases |")
    print("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |")
    for row in rows:
        print(
            f"| {row['name']} | {observed(row['server_wait_p99'])} / "
            f"{observed(row['server_service_p99'])} | {row['advanced']} | "
            f"{row['issues']} / {row['completions']} | "
            f"{row['joined']} / {row['server_requests']} | {observed(row['kernel_p99'])} | "
            f"{row['extra_read_bytes']} / {row['extra_write_bytes']} ({row['udev_bytes']}) | "
            f"{row['programs']} / {row['copies']} / {row['erases']} |"
        )


if __name__ == "__main__":
    capture = Path(sys.argv[1])
    rows = [case(config.parent) for config in sorted(capture.glob("*/case.tsv"))]
    if not rows:
        raise SystemExit("no benchmark cases")
    report(rows)
