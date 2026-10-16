import shutil
import subprocess

import pytest

from conftest import FIXTURE_CACHE
from houritsu_builder.build import build
from houritsu_builder.cache import Cache


def git(repo, *args):
    return subprocess.run(["git", *args], cwd=repo, capture_output=True, text=True, check=True).stdout


@pytest.fixture
def cache(tmp_path):
    shutil.copytree(FIXTURE_CACHE, tmp_path / "cache")
    return Cache(tmp_path / "cache")


def test_builds_history_from_fixture(cache, tmp_path):
    report = build(cache, tmp_path / "out", lint=False)
    out = tmp_path / "out"
    subjects = git(out, "log", "--topo-order", "--format=%ad %s", "--date=short", "main").splitlines()
    assert len(subjects) == 4  # root + 3 amendments
    assert subjects[-1].endswith("リポジトリの初期化")
    assert [s.split()[0] for s in subjects[:3]] == ["2020-01-01", "2019-05-01", "2016-01-01"]
    md = git(out, "show", "main:laws/act/323AC1000000178.md")
    assert md.startswith('---\nlaw_id: "323AC1000000178"\n')
    assert "# 国民の祝日に関する法律" in md
    assert report["commits_main"] == 3 and report["excluded"] == []
    assert subprocess.run(["git", "rev-parse", "--verify", "-q", "upcoming"], cwd=out).returncode != 0
    assert (out / "laws" / "act" / "323AC1000000178.md").exists()  # working tree checked out


def test_build_is_deterministic(cache, tmp_path):
    build(cache, tmp_path / "a", lint=False)
    build(cache, tmp_path / "b", lint=False)
    assert git(tmp_path / "a", "rev-parse", "main") == git(tmp_path / "b", "rev-parse", "main")


def test_refuses_unknown_nonempty_dir(cache, tmp_path):
    out = tmp_path / "out"
    out.mkdir()
    (out / "precious.txt").write_text("keep")
    with pytest.raises(SystemExit, match="not a previous build"):
        build(cache, out, lint=False)
    assert (out / "precious.txt").read_text() == "keep"


def test_rebuild_replaces_previous_build(cache, tmp_path):
    build(cache, tmp_path / "out", lint=False)
    build(cache, tmp_path / "out", lint=False)
    assert len(git(tmp_path / "out", "log", "--format=%s", "main").splitlines()) == 4


@pytest.mark.skipif(shutil.which("npx") is None, reason="needs node/npx for markdownlint-cli2")
def test_markdownlint_passes_on_every_revision(cache, tmp_path):
    report = build(cache, tmp_path / "out", lint=True)
    assert report["lint"] == "ok"
