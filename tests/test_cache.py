import json

from houritsu_builder.cache import Cache


def test_layout(tmp_path):
    c = Cache(tmp_path)
    assert c.laws_path() == tmp_path / "laws.json.gz"
    assert c.revisions_path("415AC0000000057") == tmp_path / "revisions" / "415AC0000000057.json.gz"
    rid = "415AC0000000057_20261001_507AC0000000070"
    assert c.data_path(rid) == tmp_path / "data" / "415AC0000000057" / f"{rid}.json.gz"


def test_write_read_roundtrip_and_no_temp_left(tmp_path):
    c = Cache(tmp_path)
    p = c.revisions_path("X")
    c.write(p, {"a": "全角　空白"})
    assert c.read(p) == {"a": "全角　空白"}
    assert [f.name for f in p.parent.iterdir()] == ["X.json.gz"]


def test_record_failure_appends(tmp_path):
    c = Cache(tmp_path)
    c.record_failure("law_data", "R1", "boom")
    c.record_failure("law_revisions", "L1", "boom2")
    lines = (tmp_path / "failures.jsonl").read_text().splitlines()
    assert [json.loads(l)["key"] for l in lines] == ["R1", "L1"]


def test_revision_files_sorted(tmp_path):
    c = Cache(tmp_path)
    for law in ("B", "A"):
        c.write(c.revisions_path(law), {})
    assert [p.name for p in c.revision_files()] == ["A.json.gz", "B.json.gz"]
