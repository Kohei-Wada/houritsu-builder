from houritsu_builder.api import ApiError
from houritsu_builder.cache import Cache
from houritsu_builder.fetch import fetch, fetch_law_list


class FakeClient:
    def __init__(self, routes):
        self.routes = routes
        self.calls = []

    def get_json(self, path, **params):
        self.calls.append(path)
        value = self.routes[path]
        if isinstance(value, Exception):
            raise value
        return value(params) if callable(value) else value


def law(law_id, law_type="Act"):
    return {"law_info": {"law_id": law_id, "law_type": law_type}}


def revs(law_id, *rids):
    return {"law_info": {"law_id": law_id}, "revisions": [{"law_revision_id": r} for r in rids]}


def test_law_list_paginates():
    pages = {0: {"total_count": 3, "laws": [law("A"), law("B")]}, 2: {"total_count": 3, "laws": [law("C")]}}
    client = FakeClient({"laws": lambda p: pages[p["offset"]]})
    assert [l["law_info"]["law_id"] for l in fetch_law_list(client)] == ["A", "B", "C"]


def test_fetch_filters_skips_cached_and_records_failures(tmp_path):
    cache = Cache(tmp_path)
    cache.write(cache.data_path("A_1_X"), {"cached": True})
    client = FakeClient({
        "laws": {"total_count": 3, "laws": [law("A"), law("B"), law("O", "CabinetOrder")]},
        "law_revisions/A": revs("A", "A_1_X", "A_2_Y"),
        "law_revisions/B": ApiError("gave up"),
        "law_data/A_2_Y": {"law_full_text": "x"},
    })
    stats = fetch(client, cache, law_type="Act", log=lambda *a: None)
    assert "law_data/A_1_X" not in client.calls
    assert "law_revisions/O" not in client.calls
    assert cache.read(cache.data_path("A_2_Y")) == {"law_full_text": "x"}
    assert stats == {"laws": 1, "revisions": 2, "fetched": 1, "failed": 1}
    assert "law_revisions" in (tmp_path / "failures.jsonl").read_text()


def test_fetch_by_law_id_skips_listing(tmp_path):
    client = FakeClient({"law_revisions/A": revs("A"), "laws": AssertionError("must not list")})
    fetch(client, Cache(tmp_path), law_ids=["A"], log=lambda *a: None)
    assert client.calls == ["law_revisions/A"]
