import datetime
import hashlib
import os
import zlib
from pathlib import Path
from typing import BinaryIO

COMMITTER = "houritsu-builder <houritsu-builder@users.noreply.github.com>"
_JST = datetime.timezone(datetime.timedelta(hours=9))


def jst_midnight(date: str) -> int:
    d = datetime.date.fromisoformat(date)
    return max(86400, int(datetime.datetime(d.year, d.month, d.day, tzinfo=_JST).timestamp()))


def write_blob(objects: Path, data: bytes) -> str:
    """Store data as a loose git blob, so compression runs in parallel workers instead of in fast-import."""
    raw = b"blob %d\0" % len(data) + data
    sha = hashlib.sha1(raw).hexdigest()
    path = objects / sha[:2] / sha[2:]
    if not path.exists():
        path.parent.mkdir(parents=True, exist_ok=True)
        tmp = path.with_name(f"{sha[2:]}.{os.getpid()}.tmp")
        tmp.write_bytes(zlib.compress(raw, 1))
        os.replace(tmp, path)
    return sha


class Stream:
    def __init__(self, out: BinaryIO):
        self.out = out
        self.mark = 0
        self.marks: dict[str, int] = {}

    def _data(self, text: str) -> None:
        raw = text.encode()
        self.out.write(b"data %d\n" % len(raw) + raw + b"\n")

    def commit(self, ref: str, date: str, message: str, files: dict[str, str], from_ref: str | None = None,
               deleted: set[str] = frozenset(), author: str | None = None, blobs: dict[str, str] | None = None) -> int:
        self.mark += 1
        ts = jst_midnight(date)
        self.out.write(f"commit {ref}\nmark :{self.mark}\nauthor {f"{author} <>" if author else COMMITTER} {ts} +0900\ncommitter {COMMITTER} {ts} +0900\n".encode())
        self._data(message)
        if from_ref:
            parent = f":{self.marks[from_ref]}" if from_ref in self.marks else from_ref
            self.out.write(f"from {parent}\n".encode())
        for path in sorted(deleted):
            self.out.write(f"D {path}\n".encode())
        blobs = blobs or {}
        for path in sorted(files.keys() | blobs.keys()):
            if path in blobs:
                self.out.write(f"M 100644 {blobs[path]} {path}\n".encode())
            else:
                self.out.write(f"M 100644 inline {path}\n".encode())
                self._data(files[path])
        self.out.write(b"\n")
        self.marks[ref] = self.mark
        return self.mark

    def done(self) -> None:
        self.out.write(b"done\n")
