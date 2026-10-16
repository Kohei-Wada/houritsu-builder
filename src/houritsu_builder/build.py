import base64
import collections
import json
import shutil
import subprocess
import tempfile
from importlib import resources
from pathlib import Path

from houritsu_builder.cache import Cache
from houritsu_builder.fastimport import Stream
from houritsu_builder.fidelity import check
from houritsu_builder.frontmatter import frontmatter
from houritsu_builder.history import load_revisions, plan
from houritsu_builder.render import render_law

MARKER = ".git/houritsu-build"


def _asset(name: str) -> str:
    return resources.files("houritsu_builder").joinpath("assets", name).read_text()


def _prepare(out: Path) -> None:
    if out.exists() and any(out.iterdir()):
        if not (out / MARKER).exists():
            raise SystemExit(f"{out} is not empty and not a previous build; refusing to touch it")
        shutil.rmtree(out)
    out.mkdir(parents=True, exist_ok=True)
    subprocess.run(["git", "init", "-q", "-b", "main", str(out)], check=True)
    (out / MARKER).write_text("created by houritsu-builder\n")


def build(cache: Cache, out: Path, law_type: str | None = None, lint: bool = True) -> dict:
    out = Path(out)
    _prepare(out)
    revisions, excluded = load_revisions(cache, law_type)
    main_groups, upcoming_groups = plan(revisions)
    unknown = collections.Counter()
    lint_dir = Path(tempfile.mkdtemp(prefix="houritsu-lint-")) if lint else None

    proc = subprocess.Popen(["git", "fast-import", "--quiet", "--done"], cwd=out, stdin=subprocess.PIPE)
    stream = Stream(proc.stdin)
    root_date = main_groups[0].commit_date if main_groups else "1970-01-02"
    stream.commit("refs/heads/main", root_date, "リポジトリの初期化\n",
                  {"README.md": _asset("README.md"), ".markdownlint-cli2.yaml": _asset("markdownlint-cli2.yaml")})
    files = 0
    for ref, groups in (("refs/heads/main", main_groups), ("refs/heads/upcoming", upcoming_groups)):
        for i, group in enumerate(groups):
            changed = {}
            for r in group.revisions:
                xml = base64.b64decode(cache.read(cache.data_path(r.revision_id))["law_full_text"])
                rendered = render_law(xml)
                check(xml, rendered.markdown, r.revision_id)
                unknown.update(rendered.unknown)
                changed[r.path] = frontmatter(r.law_info, r.rev) + rendered.markdown
                if lint_dir:
                    target = lint_dir / "laws" / f"{r.revision_id}.md"
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_text(changed[r.path])
                files += 1
            from_ref = "refs/heads/main" if ref.endswith("upcoming") and i == 0 else None
            stream.commit(ref, group.commit_date, group.message(), changed, from_ref=from_ref)
    stream.done()
    proc.stdin.close()
    if proc.wait() != 0:
        raise SystemExit("git fast-import failed")
    subprocess.run(["git", "reset", "-q", "--hard", "main"], cwd=out, check=True)

    report = {
        "commits_main": len(main_groups),
        "commits_upcoming": len(upcoming_groups),
        "files": files,
        "excluded": excluded,
        "unknown": dict(unknown.most_common()),
        "lint": "skipped",
    }
    if lint_dir:
        (lint_dir / ".markdownlint-cli2.yaml").write_text(_asset("markdownlint-cli2.yaml"))
        result = subprocess.run(["npx", "--yes", "markdownlint-cli2"], cwd=lint_dir, capture_output=True, text=True)
        shutil.rmtree(lint_dir)
        if result.returncode != 0:
            raise SystemExit("markdownlint failed:\n" + (result.stdout + result.stderr)[-4000:])
        report["lint"] = "ok"
    cache.root.mkdir(parents=True, exist_ok=True)
    (cache.root / "build-report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2))
    return report


def run(args) -> int:
    report = build(Cache(args.cache), args.out, law_type=args.law_type, lint=not args.no_lint)
    print(json.dumps({k: v for k, v in report.items() if k != "unknown"}, ensure_ascii=False))
    print("unknown elements:", report["unknown"])
    return 0
