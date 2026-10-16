import shutil
import subprocess

import pytest

from conftest import FIXTURE_CACHE
from houritsu_builder.build import RUMDL, build
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
    md = git(out, "show", "main:laws/act/323/323AC1000000178.md")
    assert md.startswith('---\nlaw_id: "323AC1000000178"\n')
    assert "# 国民の祝日に関する法律" in md
    assert report["commits_main"] == 3 and report["excluded"] == [] and report["skipped"] == []
    assert subprocess.run(["git", "rev-parse", "--verify", "-q", "upcoming"], cwd=out).returncode != 0
    assert (out / "laws" / "act" / "323" / "323AC1000000178.md").exists()  # working tree checked out


def test_build_is_deterministic(cache, tmp_path):
    build(cache, tmp_path / "a", lint=False, jobs=1)
    build(cache, tmp_path / "b", lint=False, jobs=4)
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


def test_rebuild_keeps_remote_and_tracking_config(cache, tmp_path):
    out = tmp_path / "out"
    build(cache, out, lint=False)
    git(out, "remote", "add", "origin", "git@example.com:x/houritsu.git")
    git(out, "config", "branch.main.remote", "origin")
    build(cache, out, lint=False)
    assert git(out, "remote", "get-url", "origin") == "git@example.com:x/houritsu.git\n"
    assert git(out, "config", "branch.main.remote") == "origin\n"


def test_rumdl_passes_on_every_revision(cache, tmp_path):
    report = build(cache, tmp_path / "out", lint=True)
    assert report["lint"] == "ok"


def _spawned(monkeypatch):
    import houritsu_builder.build as b

    procs = []
    real = subprocess.Popen

    def spy(*a, **k):
        p = real(*a, **k)
        procs.append(p)
        return p

    monkeypatch.setattr(b.subprocess, "Popen", spy)
    return procs


def test_lint_failure_keeps_previous_build_and_leaves_no_staging(cache, tmp_path, monkeypatch):
    out = tmp_path / "out"
    build(cache, out, lint=False)
    before = git(out, "rev-parse", "main")
    real_run = subprocess.run

    def fake_run(cmd, *a, **k):
        if cmd[0] == RUMDL:
            return subprocess.CompletedProcess(cmd, 1, stdout="x" * 5000, stderr="")
        return real_run(cmd, *a, **k)

    monkeypatch.setattr("houritsu_builder.build.subprocess.run", fake_run)
    with pytest.raises(SystemExit, match="rumdl failed"):
        build(cache, out, lint=True, jobs=1)
    assert git(out, "rev-parse", "main") == before
    assert not (tmp_path / "out.building").exists()
    assert not (cache.root / "lint").exists()
    assert len((cache.root / "lint.log").read_text()) == 5000


def test_fidelity_error_cleans_up(cache, tmp_path, monkeypatch):
    from houritsu_builder.fidelity import FidelityError

    procs = _spawned(monkeypatch)

    def boom(xml, md, label):
        raise FidelityError("boom")

    monkeypatch.setattr("houritsu_builder.build.check", boom)
    with pytest.raises(FidelityError):
        build(cache, tmp_path / "out", lint=True, jobs=1)
    assert procs and all(p.returncode is not None for p in procs)
    assert not (tmp_path / "out").exists()
    assert not (tmp_path / "out.building").exists()
    assert not (cache.root / "lint").exists()


def test_upcoming_branch_descends_from_main(cache, tmp_path):
    import copy

    from conftest import LAW_ID

    rev_file = cache.revisions_path(LAW_ID)
    doc = cache.read(rev_file)
    last = doc["revisions"][-1]
    new = copy.deepcopy(last)
    new["law_revision_id"] = f"{LAW_ID}_20300101_999AC0000000001"
    new["amendment_enforcement_date"] = "2030-01-01"
    new["amendment_law_id"] = "999AC0000000001"
    new["current_revision_status"] = "UnEnforced"
    doc["revisions"].append(new)
    cache.write(rev_file, doc)
    cache.write(cache.data_path(new["law_revision_id"]), cache.read(cache.data_path(last["law_revision_id"])))

    out = tmp_path / "out"
    report = build(cache, out, lint=False)
    assert report["commits_upcoming"] == 1
    subprocess.run(["git", "merge-base", "--is-ancestor", "main", "upcoming"], cwd=out, check=True)
    assert "999AC0000000001" not in git(out, "log", "--format=%B", "main")
    assert "999AC0000000001" in git(out, "log", "--format=%B", "-1", "upcoming")


def test_repeal_removes_the_file(cache, tmp_path):
    from conftest import LAW_ID

    rev_file = cache.revisions_path(LAW_ID)
    doc = cache.read(rev_file)
    doc["revisions"].insert(0, dict(doc["revisions"][0], law_revision_id=f"{LAW_ID}_20250401_999AC0000000001",
                                    amendment_enforcement_date="2025-04-01", amendment_law_id="999AC0000000001",
                                    amendment_law_title="国民の祝日に関する法律を廃止する法律",
                                    current_revision_status="Repeal", repeal_status="Repeal"))
    cache.write(rev_file, doc)
    out = tmp_path / "out"
    report = build(cache, out, lint=False)
    assert report["commits_main"] == 4
    assert git(out, "log", "-1", "--format=%s", "main") == "国民の祝日に関する法律を廃止する法律 施行\n"
    assert git(out, "ls-tree", "-r", "--name-only", "main", "laws") == ""
    assert not (out / "laws" / "act" / "323" / f"{LAW_ID}.md").exists()
    assert len(git(out, "log", "--format=%h", "main", "--", f"laws/act/323/{LAW_ID}.md").split()) == 4


def test_worker_error_propagates_and_cleans_up(cache, tmp_path):
    import base64
    import xml.etree.ElementTree as ET

    from conftest import LAW_ID

    rid = cache.read(cache.revisions_path(LAW_ID))["revisions"][0]["law_revision_id"]
    doc = cache.read(cache.data_path(rid))
    doc["law_full_text"] = base64.b64encode(b"<Law>").decode()
    cache.write(cache.data_path(rid), doc)
    with pytest.raises(ET.ParseError):
        build(cache, tmp_path / "out", lint=False, jobs=2)
    assert not (tmp_path / "out.building").exists()
