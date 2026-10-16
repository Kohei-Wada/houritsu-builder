import gzip
import json
import os
from pathlib import Path
from typing import Any


class Cache:
    def __init__(self, root: Path):
        self.root = Path(root)

    def laws_path(self) -> Path:
        return self.root / "laws.json.gz"

    def revisions_path(self, law_id: str) -> Path:
        return self.root / "revisions" / f"{law_id}.json.gz"

    def data_path(self, revision_id: str) -> Path:
        law_id = revision_id.split("_", 1)[0]
        return self.root / "data" / law_id / f"{revision_id}.json.gz"

    def write(self, path: Path, obj: Any) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        tmp = path.with_name(path.name + ".tmp")
        raw = json.dumps(obj, ensure_ascii=False).encode()
        with open(tmp, "wb") as f:
            f.write(gzip.compress(raw, mtime=0))
        os.replace(tmp, path)

    def read(self, path: Path) -> Any:
        return json.loads(gzip.decompress(path.read_bytes()))

    def record_failure(self, kind: str, key: str, error: str) -> None:
        self.root.mkdir(parents=True, exist_ok=True)
        with open(self.root / "failures.jsonl", "a") as f:
            f.write(json.dumps({"kind": kind, "key": key, "error": error}, ensure_ascii=False) + "\n")

    def revision_files(self) -> list[Path]:
        d = self.root / "revisions"
        return sorted(d.glob("*.json.gz")) if d.exists() else []

    def not_found(self) -> set[str]:
        path = self.root / "failures.jsonl"
        if not path.exists():
            return set()
        found = set()
        for line in path.read_text(encoding="utf-8").splitlines():
            if not line.strip():
                continue
            f = json.loads(line)
            if f.get("kind") == "law_data" and str(f.get("error", "")).startswith("404"):
                found.add(f["key"])
        return found
