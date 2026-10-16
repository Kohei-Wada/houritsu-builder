import base64
import collections
import json
import shutil
import subprocess
from importlib import resources
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


def _stream(cache: Cache, staging: Path, main_groups, upcoming_groups, lint_dir: Path | None, unknown) -> int:
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
                    xml = base64.b64decode(cache.read(cache.data_path(r.revision_id))["law_full_text"])
                    rendered = render_law(xml)
                    check(xml, rendered.markdown, r.revision_id)
                    unknown.update(rendered.unknown)
                    changed[r.path] = frontmatter(r.law_info, r.rev) + rendered.markdown
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


def build(cache: Cache, out: Path, law_type: str | None = None, lint: bool = True) -> dict:
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
        files = _stream(cache, staging, main_groups, upcoming_groups, lint_dir if lint else None, unknown)
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
