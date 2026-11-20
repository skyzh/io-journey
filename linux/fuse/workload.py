import os
from pathlib import Path
import struct
import sys
import zlib


def frame(data):
    return struct.pack("<II", len(data), zlib.crc32(data)) + data


def sync_directory(directory):
    fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


action, directory = sys.argv[1:]
root = Path(directory)
log = root / "events.log"
original = frame(b"first") + frame(b"second")
replacement = frame(b"replace" * 585 + b"12345678")

if action == "append":
    with log.open("xb") as output:
        output.write(frame(b"first"))
    with log.open("ab") as output:
        output.write(frame(b"second"))
        output.flush()
        os.fsync(output.fileno())
    sync_directory(root)
    assert log.read_bytes() == original
    print("FUSE appended and synced first and second.")
elif action == "tail":
    tail = frame(b"volatile")
    with log.open("ab") as output:
        output.write(tail)
    assert log.read_bytes() == original + tail
elif action == "verify":
    assert list(root.iterdir()) == [log]
    assert log.read_bytes() == original
    print("FUSE reopened first and second after the unsynced tail.")
elif action == "replace":
    staged = root / ".events.tmp"
    with staged.open("xb") as output:
        output.write(replacement)
        output.flush()
        os.fsync(output.fileno())
    with log.open("rb") as prior:
        os.replace(staged, log)
        sync_directory(root)
        assert prior.read() == original
        assert os.fstat(prior.fileno()).st_nlink == 0
    assert log.read_bytes() == replacement
    print("FUSE synced replacement and retained the prior open file until close.")
elif action == "truncate":
    with log.open("r+b") as output:
        output.truncate(3)
        output.seek(0)
        assert output.read() == replacement[:3]
        output.truncate(7)
        output.seek(0)
        assert output.read() == replacement[:3] + bytes(4)
elif action == "verify-replacement":
    assert list(root.iterdir()) == [log]
    assert log.read_bytes() == replacement
    assert log.stat().st_blocks == 16
    print("FUSE reopened the complete synced replacement after unsynced truncate.")
else:
    raise ValueError(f"unknown workload action {action}")
