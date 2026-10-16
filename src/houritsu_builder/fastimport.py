import datetime
from typing import BinaryIO

COMMITTER = "houritsu-builder <houritsu-builder@users.noreply.github.com>"
_JST = datetime.timezone(datetime.timedelta(hours=9))


def jst_midnight(date: str) -> int:
    d = datetime.date.fromisoformat(date)
    return max(86400, int(datetime.datetime(d.year, d.month, d.day, tzinfo=_JST).timestamp()))


class Stream:
    def __init__(self, out: BinaryIO):
        self.out = out
        self.mark = 0
        self.marks: dict[str, int] = {}

    def _data(self, text: str) -> None:
        raw = text.encode()
        self.out.write(b"data %d\n" % len(raw) + raw + b"\n")

    def commit(self, ref: str, date: str, message: str, files: dict[str, str], from_ref: str | None = None,
               deleted: set[str] = frozenset(), author: str | None = None) -> int:
        self.mark += 1
        ts = jst_midnight(date)
        self.out.write(f"commit {ref}\nmark :{self.mark}\nauthor {f"{author} <>" if author else COMMITTER} {ts} +0900\ncommitter {COMMITTER} {ts} +0900\n".encode())
        self._data(message)
        if from_ref:
            parent = f":{self.marks[from_ref]}" if from_ref in self.marks else from_ref
            self.out.write(f"from {parent}\n".encode())
        for path in sorted(deleted):
            self.out.write(f"D {path}\n".encode())
        for path in sorted(files):
            self.out.write(f"M 100644 inline {path}\n".encode())
            self._data(files[path])
        self.out.write(b"\n")
        self.marks[ref] = self.mark
        return self.mark

    def done(self) -> None:
        self.out.write(b"done\n")
