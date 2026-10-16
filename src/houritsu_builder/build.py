import base64
import collections
import json
import os
import shutil
import subprocess
from concurrent.futures import ProcessPoolExecutor
from importlib import resources
from itertools import islice
from pathlib import Path

from houritsu_builder.cache import Cache
from houritsu_builder.fastimport import Stream
from houritsu_builder.fidelity import check
from houritsu_builder.frontmatter import frontmatter
from houritsu_builder.history import load_revisions, plan
from houritsu_builder.render import render_law

MARKER = ".git/houritsu-build"
MARKDOWNLINT = ["npx", "--yes", "markdownlint-cli2@0.23.3"]


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


def _render(root: Path, revision_id: str) -> tuple[str, collections.Counter]:
    cache = Cache(root)
    xml = base64.b64decode(cache.read(cache.data_path(revision_id))["law_full_text"])
    rendered = render_law(xml)
    check(xml, rendered.markdown, revision_id)
    return rendered.markdown, rendered.unknown


def _rendered_in_order(cache: Cache, revision_ids, jobs: int):
    if jobs <= 1:
        yield from (_render(cache.root, rid) for rid in revision_ids)
        return
    ids = iter(revision_ids)
    with ProcessPoolExecutor(jobs) as pool:
        window = collections.deque(pool.submit(_render, cache.root, rid) for rid in islice(ids, jobs * 4))
        while window:
            result = window.popleft().result()
            window.extend(pool.submit(_render, cache.root, rid) for rid in islice(ids, 1))
            yield result


def _stream(cache: Cache, staging: Path, main_groups, upcoming_groups, lint_dir: Path | None, unknown, jobs: int) -> int:
    rendered = _rendered_in_order(cache, [r.revision_id for g in main_groups + upcoming_groups
                                          for r in g.revisions if not r.repealed], jobs)
    proc = subprocess.Popen(["git", "fast-import", "--quiet", "--done"], cwd=staging, stdin=subprocess.PIPE)
    files = 0
    try:
        stream = Stream(proc.stdin)
        root_date = main_groups[0].commit_date if main_groups else "1970-01-02"
        stream.commit("refs/heads/main", root_date, "リポジトリの初期化\n",
                      {"README.md": _asset("README.md"), ".markdownlint-cli2.yaml": _asset("markdownlint-cli2.yaml")})
        for ref, groups in (("refs/heads/main", main_groups), ("refs/heads/upcoming", upcoming_groups)):
            for i, group in enumerate(groups):
                changed, deleted = {}, set()
                for r in group.revisions:
                    if r.repealed:
                        deleted.add(r.path)
                        continue
                    markdown, found = next(rendered)
                    unknown.update(found)
                    changed[r.path] = frontmatter(r.law_info, r.rev) + markdown
                    if lint_dir:
                        target = lint_dir / r.path.removesuffix(".md") / f"{r.revision_id}.md"
                        target.parent.mkdir(parents=True, exist_ok=True)
                        target.write_text(changed[r.path])
                    files += 1
                from_ref = "refs/heads/main" if ref.endswith("upcoming") and i == 0 else None
                stream.commit(ref, group.commit_date, group.message(), changed, from_ref=from_ref, deleted=deleted, author=group.author)
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
    return files


def _copy_remote_config(old: Path, new: Path) -> None:
    if not (old / ".git").exists():
        return
    found = subprocess.run(["git", "config", "--local", "--get-regexp", r"^(remote|branch)\."], cwd=old, capture_output=True, text=True)
    for line in found.stdout.splitlines():
        key, value = line.split(" ", 1)
        subprocess.run(["git", "config", "--add", key, value], cwd=new, check=True)


def build(cache: Cache, out: Path, law_type: str | None = None, lint: bool = True, jobs: int | None = None) -> dict:
    out = Path(out)
    staging = out.with_name(out.name + ".building")
    if out.exists() and any(out.iterdir()) and not (out / MARKER).exists():
        raise SystemExit(f"{out} is not empty and not a previous build; refusing to touch it")
    _prepare(staging)
    lint_dir = cache.root / "lint"
    try:
        revisions, excluded, skipped = load_revisions(cache, law_type)
        main_groups, upcoming_groups = plan(revisions)
        unknown = collections.Counter()
        if lint:
            shutil.rmtree(lint_dir, ignore_errors=True)
            lint_dir.mkdir(parents=True)
        files = _stream(cache, staging, main_groups, upcoming_groups, lint_dir if lint else None, unknown,
                        jobs or os.cpu_count() or 1)
        subprocess.run(["git", "reset", "-q", "--hard", "main"], cwd=staging, check=True)

        report = {
            "commits_main": len(main_groups),
            "commits_upcoming": len(upcoming_groups),
            "files": files,
            "excluded": excluded,
            "skipped": skipped,
            "unknown": dict(unknown.most_common()),
            "lint": "skipped",
        }
        if lint:
            (lint_dir / ".markdownlint-cli2.yaml").write_text(_asset("markdownlint-cli2.yaml"))
            result = subprocess.run(MARKDOWNLINT, cwd=lint_dir, capture_output=True, text=True)
            output = result.stdout + result.stderr
            if result.returncode != 0:
                (cache.root / "lint.log").write_text(output)
                raise SystemExit("markdownlint failed (full log: lint.log in cache):\n" + output[:4000])
            report["lint"] = "ok"
        _copy_remote_config(out, staging)
        _clear(out)
        out.rmdir()
        staging.rename(out)
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
