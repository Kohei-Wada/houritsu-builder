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


def test_fetch_takes_never_fetched_laws_first(tmp_path):
    cache = Cache(tmp_path)
    cache.write(cache.revisions_path("A"), revs("A"))
    client = FakeClient({
        "laws": {"total_count": 2, "laws": [law("A"), law("B")]},
        "law_revisions/A": revs("A"),
        "law_revisions/B": revs("B"),
    })
    fetch(client, cache, log=lambda m: None, full=True)
    assert client.calls == ["laws", "law_revisions/B", "law_revisions/A"]


def test_fetch_in_parallel_gets_everything(tmp_path):
    ids = [f"L{n}" for n in range(20)]
    routes = {"laws": {"total_count": 20, "laws": [law(i) for i in ids]}}
    for i in ids:
        routes[f"law_revisions/{i}"] = revs(i, f"{i}_1_X", f"{i}_2_Y")
        routes[f"law_data/{i}_1_X"] = routes[f"law_data/{i}_2_Y"] = {"law_full_text": i}
    routes["law_revisions/L3"] = ApiError("gave up")
    cache = Cache(tmp_path)
    stats = fetch(FakeClient(routes), cache, log=lambda m: None, jobs=4)
    assert stats == {"laws": 19, "revisions": 38, "fetched": 38, "failed": 1}
    assert cache.read(cache.data_path("L7_2_Y")) == {"law_full_text": "L7"}


def listed(law_id, rid, updated="u1"):
    return {"law_info": {"law_id": law_id, "law_type": "Act"}, "revision_info": {"law_revision_id": rid, "updated": updated}}


def rev(rid, amendment=None, status="CurrentEnforced", enforced="2020-01-01", updated="u1"):
    return {"law_revision_id": rid, "amendment_law_id": amendment, "current_revision_status": status,
            "amendment_enforcement_date": enforced, "updated": updated}


def law_lists(now, future, by_amendment=None):
    def route(p):
        if "amendment_law_id" in p:
            found = (by_amendment or {}).get(p["amendment_law_id"], [])
        else:
            found = future if p.get("asof") else now
        return {"total_count": len(found), "laws": found}
    return route


def test_delta_refetches_only_laws_that_may_have_changed(tmp_path):
    cache = Cache(tmp_path)
    for law_id, revisions in {
        "SAME": [rev("SAME_1")],
        "NEWER": [rev("NEWER_1")],
        "EDITED": [rev("EDITED_1")],
        "DUE": [rev("DUE_2", status="UnEnforced", enforced="2000-01-01"), rev("DUE_1")],
        "LATER": [rev("LATER_2", status="UnEnforced", enforced="9999-01-01"), rev("LATER_1")],
    }.items():
        cache.write(cache.revisions_path(law_id), {"law_info": {"law_id": law_id}, "revisions": revisions})
    now = [listed("SAME", "SAME_1"), listed("NEWER", "NEWER_1"), listed("EDITED", "EDITED_1", "u2"),
           listed("DUE", "DUE_1"), listed("LATER", "LATER_1"), listed("NEW", "NEW_1")]
    future = [listed("SAME", "SAME_1"), listed("NEWER", "NEWER_2"), listed("EDITED", "EDITED_1", "u2"),
              listed("DUE", "DUE_2"), listed("LATER", "LATER_2"), listed("NEW", "NEW_1")]
    routes = {"laws": law_lists(now, future)}
    for law_id in ("NEWER", "EDITED", "DUE", "NEW"):
        routes[f"law_revisions/{law_id}"] = {"law_info": {"law_id": law_id}, "revisions": []}
    client = FakeClient(routes)
    fetch(client, cache, log=lambda m: None)
    assert sorted(c for c in client.calls if c != "laws") == ["law_revisions/DUE", "law_revisions/EDITED", "law_revisions/NEW", "law_revisions/NEWER"]


def test_delta_follows_a_new_amending_law_to_the_other_laws_it_amends(tmp_path):
    cache = Cache(tmp_path)
    cache.write(cache.revisions_path("A"), {"law_info": {"law_id": "A"}, "revisions": [rev("A_1", "X1")]})
    cache.write(cache.revisions_path("B"), {"law_info": {"law_id": "B"}, "revisions": [rev("B_3", "X3"), rev("B_1", "X1")]})
    now = [listed("A", "A_1"), listed("B", "B_1")]
    future = [listed("A", "A_2"), listed("B", "B_3")]
    client = FakeClient({
        "laws": law_lists(now, future, {"X2": [listed("A", "A_2"), listed("B", "B_3")]}),
        "law_revisions/A": {"law_info": {"law_id": "A"}, "revisions": [rev("A_2", "X2"), rev("A_1", "X1")]},
        "law_revisions/B": {"law_info": {"law_id": "B"}, "revisions": [rev("B_3", "X3"), rev("B_2", "X2"), rev("B_1", "X1")]},
        "law_data/A_2": {"law_full_text": "a"},
        "law_data/B_2": {"law_full_text": "b"},
        "law_data/A_1": {"law_full_text": "a"},
        "law_data/B_1": {"law_full_text": "b"},
        "law_data/B_3": {"law_full_text": "b"},
    })
    fetch(client, cache, log=lambda m: None)
    assert cache.read(cache.data_path("B_2")) == {"law_full_text": "b"}
    assert client.calls.count("law_revisions/A") == 1 and client.calls.count("law_revisions/B") == 1
