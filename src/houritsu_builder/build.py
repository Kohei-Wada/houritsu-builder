import base64
import collections
import hashlib
import importlib.metadata
import json
import os
import shutil
import subprocess
import sysconfig
from concurrent.futures import ProcessPoolExecutor
from importlib import resources
from itertools import islice
from pathlib import Path

from houritsu_builder.cache import Cache
from houritsu_builder.fastimport import Stream, write_blob
from houritsu_builder.fidelity import check
from houritsu_builder.frontmatter import frontmatter
from houritsu_builder.history import Revision, load_revisions, plan
from houritsu_builder.render import render_law

MARKER = ".git/houritsu-build"
RENDERED = "rendered.json"  # revision → blob of the last build, reused while the inputs are unchanged
RUMDL = str(Path(sysconfig.get_path("scripts")) / "rumdl")


def _asset(name: str) -> str:
    return resources.files("houritsu_builder").joinpath("assets", name).read_text()


def _clear(path: Path) -> None:
    if path.exists() and any(path.iterdir()):
        if not (path / MARKER).exists():
            raise SystemExit(f"{path} is not empty and not a previous build; refusing to touch it")
        shutil.rmtree(path)
    path.mkdir(parents=True, exist_ok=True)


def _prepare(staging: Path) -> None:
    _clear(staging)
    subprocess.run(["git", "init", "-q", "-b", "main", str(staging)], check=True)
    (staging / MARKER).write_text("created by houritsu-builder\n")


def _code_version() -> str:
    h = hashlib.sha1(importlib.metadata.version("rumdl").encode())
    pkg = Path(__file__).parent
    for p in sorted(pkg.rglob("*")):
        if p.is_file() and p.suffix in {".py", ".md", ".toml"}:
            h.update(p.relative_to(pkg).as_posix().encode() + b"\0" + p.read_bytes())
    return h.hexdigest()


def _input_key(cache: Cache, r: Revision) -> str:
    st = cache.data_path(r.revision_id).stat()
    meta = json.dumps([r.law_info, r.rev], ensure_ascii=False, sort_keys=True)
    return hashlib.sha1(f"{st.st_size} {st.st_mtime_ns} {meta}".encode()).hexdigest()


def _load_rendered(cache: Cache, out: Path, version: str) -> dict:
    path = cache.root / RENDERED
    if not path.exists() or not (out / MARKER).exists():
        return {}
    doc = json.loads(path.read_text())
    if doc.get("version") != version:
        return {}
    entries = doc["entries"]
    found = subprocess.run(["git", "cat-file", "--batch-check=%(objectname)"], cwd=out, capture_output=True, text=True, check=True,
                           input="".join(e[1] + "\n" for e in entries.values()))
    present = {line for line in found.stdout.splitlines() if not line.endswith(" missing")}
    return {rid: e for rid, e in entries.items() if e[1] in present}


def _render(root: Path, objects: Path, r: Revision, lint_file: Path | None) -> tuple[str, collections.Counter]:
    cache = Cache(root)
    xml = base64.b64decode(cache.read(cache.data_path(r.revision_id))["law_full_text"])
    rendered = render_law(xml)
    check(xml, rendered.markdown, r.revision_id)
    text = frontmatter(r.law_info, r.rev) + rendered.markdown
    if lint_file:
        lint_file.parent.mkdir(parents=True, exist_ok=True)
        lint_file.write_text(text)
    return write_blob(objects, text.encode()), rendered.unknown


def _rendered_in_order(tasks, jobs: int):
    if jobs <= 1:
        yield from (_render(*t) for t in tasks)
        return
    tasks = iter(tasks)
    with ProcessPoolExecutor(jobs) as pool:
        window = collections.deque(pool.submit(_render, *t) for t in islice(tasks, jobs * 4))
        while window:
            result = window.popleft().result()
            window.extend(pool.submit(_render, *t) for t in islice(tasks, 1))
            yield result


def _stream(cache: Cache, staging: Path, main_groups, upcoming_groups, lint_dir: Path | None, unknown, jobs: int,
            previous: dict) -> tuple[int, dict]:
    todo = [r for g in main_groups + upcoming_groups for r in g.revisions if not r.repealed]
    keys = {r.revision_id: _input_key(cache, r) for r in todo}
    hits = {rid: previous[rid] for rid, key in keys.items() if rid in previous and previous[rid][0] == key}
    rendered = _rendered_in_order(((cache.root, staging / ".git" / "objects", r,
                                    lint_dir / r.path.removesuffix(".md") / f"{r.revision_id}.md" if lint_dir else None)
                                   for r in todo if r.revision_id not in hits), jobs)
    entries = {}
    proc = subprocess.Popen(["git", "fast-import", "--quiet", "--done"], cwd=staging, stdin=subprocess.PIPE)
    files = 0
    try:
        stream = Stream(proc.stdin)
        root_date = main_groups[0].commit_date if main_groups else "1970-01-02"
        stream.commit("refs/heads/main", root_date, "リポジトリの初期化\n",
                      {"README.md": _asset("README.md"), ".rumdl.toml": _asset("rumdl.toml")})
        for ref, groups in (("refs/heads/main", main_groups), ("refs/heads/upcoming", upcoming_groups)):
            for i, group in enumerate(groups):
                blobs, deleted = {}, set()
                for r in group.revisions:
                    if r.repealed:
                        deleted.add(r.path)
                        continue
                    if r.revision_id in hits:
                        _, sha, found = hits[r.revision_id]
                    else:
                        sha, found = next(rendered)
                    blobs[r.path] = sha
                    entries[r.revision_id] = [keys[r.revision_id], sha, dict(found)]
                    unknown.update(found)
                    files += 1
                from_ref = "refs/heads/main" if ref.endswith("upcoming") and i == 0 else None
                stream.commit(ref, group.commit_date, group.message(), {}, from_ref=from_ref, deleted=deleted,
                              author=group.author, blobs=blobs)
        stream.done()
    except BaseException as e:
        proc.kill()
        if isinstance(e, BrokenPipeError):
            raise SystemExit("git fast-import failed") from e
        raise
    finally:
        rendered.close()
        try:
            proc.stdin.close()
        except BrokenPipeError:
            pass
        proc.wait()
    if proc.returncode != 0:
        raise SystemExit("git fast-import failed")
    return files, entries


def _copy_remote_config(old: Path, new: Path) -> None:
    if not (old / ".git").exists():
        return
    found = subprocess.run(["git", "config", "--local", "--get-regexp", r"^(remote|branch)\."], cwd=old, capture_output=True, text=True)
    for line in found.stdout.splitlines():
        key, value = line.split(" ", 1)
        subprocess.run(["git", "config", "--add", key, value], cwd=new, check=True)


def _lint(lint_dir: Path) -> tuple[int, str]:
    (lint_dir / ".rumdl.toml").write_text(_asset("rumdl.toml"))
    result = subprocess.run([RUMDL, "check", "--no-cache", "--respect-gitignore=false", "."], cwd=lint_dir, capture_output=True, text=True)
    return result.returncode, result.stdout + result.stderr


def build(cache: Cache, out: Path, law_type: str | None = None, lint: bool = True, jobs: int | None = None) -> dict:
    out = Path(out)
    jobs = jobs or os.cpu_count() or 1
    staging = out.with_name(out.name + ".building")
    if out.exists() and any(out.iterdir()) and not (out / MARKER).exists():
        raise SystemExit(f"{out} is not empty and not a previous build; refusing to touch it")
    _prepare(staging)
    version = _code_version()
    previous = _load_rendered(cache, out, version)
    alternates = staging / ".git" / "objects" / "info" / "alternates"
    if previous:
        alternates.write_text(f"{(out / '.git' / 'objects').resolve()}\n")
    lint_dir = cache.root / "lint"
    try:
        revisions, excluded, skipped = load_revisions(cache, law_type)
        main_groups, upcoming_groups = plan(revisions)
        unknown = collections.Counter()
        if lint:
            shutil.rmtree(lint_dir, ignore_errors=True)
            lint_dir.mkdir(parents=True)
        files, entries = _stream(cache, staging, main_groups, upcoming_groups, lint_dir if lint else None, unknown, jobs, previous)
        subprocess.run(["git", "reset", "-q", "--hard", "main"], cwd=staging, check=True)
        subprocess.run(["git", "repack", "-adq"], cwd=staging, check=True)  # loose and borrowed blobs → one delta-compressed pack
        alternates.unlink(missing_ok=True)

        report = {
            "commits_main": len(main_groups),
            "commits_upcoming": len(upcoming_groups),
            "files": files,
            "rendered": sum(1 for rid in entries if previous.get(rid) != entries[rid]),
            "excluded": excluded,
            "skipped": skipped,
            "unknown": dict(unknown.most_common()),
            "lint": "skipped",
        }
        if lint:
            returncode, output = _lint(lint_dir)
            if returncode != 0:
                (cache.root / "lint.log").write_text(output)
                raise SystemExit("rumdl failed (full log: lint.log in cache):\n" + output[:4000])
            report["lint"] = "ok"
        _copy_remote_config(out, staging)
        _clear(out)
        out.rmdir()
        staging.rename(out)
        if lint:  # entries from a --no-lint build were never linted, so they must not skip lint later
            (cache.root / RENDERED).write_text(json.dumps({"version": version, "entries": entries}, ensure_ascii=False))
    except BaseException:
        shutil.rmtree(staging, ignore_errors=True)
        raise
    finally:
        shutil.rmtree(lint_dir, ignore_errors=True)
    cache.root.mkdir(parents=True, exist_ok=True)
    (cache.root / "build-report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2))
    return report


def run(args) -> int:
    report = build(Cache(args.cache), args.out, law_type=args.law_type, lint=not args.no_lint)
    print(json.dumps({k: v for k, v in report.items() if k != "unknown"}, ensure_ascii=False))
    print("unknown elements:", report["unknown"])
    return 0
