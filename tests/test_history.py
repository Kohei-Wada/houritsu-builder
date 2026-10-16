from conftest import FIXTURE_CACHE
from houritsu_builder.cache import Cache
from houritsu_builder.history import Revision, kebab, load_revisions, plan


def rev(law_id, amend, enforced, promulgated="2020-01-01", status="PreviousEnforced", title="法", amend_title="改正法"):
    rid = f"{law_id}_{(enforced or 'x').replace('-', '')}_{amend}"
    return Revision(law_id, f"{law_id}号", "Act", {"law_id": law_id, "law_num": f"{law_id}号"}, {
        "law_revision_id": rid, "law_title": title, "amendment_law_id": amend, "amendment_law_num": f"{amend}号",
        "amendment_law_title": amend_title, "amendment_enforcement_date": enforced,
        "amendment_promulgate_date": promulgated, "current_revision_status": status,
    })


def test_kebab():
    assert [kebab(t) for t in ("Act", "CabinetOrder", "MinisterialOrdinance")] == ["act", "cabinet-order", "ministerial-ordinance"]


def test_path_shards_by_era_year_and_uses_first_law_type():
    r = Revision("415AC0000000057", "", "Act", {}, {})
    assert r.path == "laws/act/415/415AC0000000057.md"
    assert Revision("322AC0000000067", "", "Act,CabinetOrder", {}, {}).path == "laws/act/322/322AC0000000067.md"


def test_one_amending_law_changing_two_laws_is_one_commit():
    main, upcoming = plan([rev("B", "Z", "2021-04-01"), rev("A", "Z", "2021-04-01"), rev("A", "Y", "2020-04-01")])
    assert [(g.amendment_law_id, [r.law_id for r in g.revisions]) for g in main] == [("Y", ["A"]), ("Z", ["A", "B"])]
    assert upcoming == []


def test_same_day_revisions_are_ordered_by_amendment_id():
    main, _ = plan([rev("A", "Q2", "2026-07-17"), rev("A", "Q1", "2026-07-17")])
    assert [g.amendment_law_id for g in main] == ["Q1", "Q2"]


def test_unknown_enforcement_date_goes_upcoming():
    main, upcoming = plan([rev("A", "Y", "2020-04-01"), rev("A", "Z", None, promulgated="2026-06-01", status="UnEnforced")])
    assert [g.amendment_law_id for g in main] == ["Y"]
    g = upcoming[0]
    assert g.commit_date == "2026-06-01"
    assert g.message().endswith("Amendment-Law-Id: Z\nEnforced: unknown\n")


def test_author_is_the_amending_law_number():
    assert plan([rev("A", "Z", "2021-04-01")])[0][0].author == "Z号"
    first = Revision("A", "平成元年法律第一号", "Act", {}, {"law_revision_id": "A_x", "amendment_enforcement_date": "2020-01-01"})
    assert plan([first])[0][0].author == "平成元年法律第一号"


def test_message_format():
    g = plan([rev("B", "Z", "2027-01-17", promulgated="2026-07-17", title="乙法"), rev("A", "Z", "2027-01-17", promulgated="2026-07-17", title="甲法")])[0][0]
    assert g.message() == (
        "改正法 施行\n\n"
        "改正法令: Z号\n"
        "公布日: 2026-07-17\n"
        "施行日: 2027-01-17\n\n"
        "変更された法令:\n"
        "- 甲法（A号）\n"
        "- 乙法（B号）\n\n"
        "Amendment-Law-Id: Z\n"
        "Enforced: 2027-01-17\n"
    )


def test_repeal_is_kept_once_and_marked_in_message():
    main, _ = plan([
        rev("A", "Y", "2020-04-01"),
        rev("A", "Z", "2021-04-01", status="Repeal"),
        rev("A", "W", "2022-04-01", status="Repeal"),
        rev("B", "Z", "2021-04-01", title="乙法"),
    ])
    assert [(g.amendment_law_id, [(r.law_id, r.repealed) for r in g.revisions]) for g in main] == [
        ("Y", [("A", False)]), ("Z", [("A", True), ("B", False)])]
    assert "- 法（A号）（廃止）\n- 乙法（B号）\n" in main[1].message()


def test_repeal_of_law_never_seen_is_dropped():
    main, _ = plan([rev("A", "Z", "2021-04-01", status="Repeal")])
    assert main == []


def test_repealed_revision_needs_no_text(tmp_path):
    import shutil

    from conftest import LAW_ID
    shutil.copytree(FIXTURE_CACHE, tmp_path / "c")
    cache = Cache(tmp_path / "c")
    doc = cache.read(cache.revisions_path(LAW_ID))
    doc["revisions"].insert(0, dict(doc["revisions"][0], law_revision_id=f"{LAW_ID}_20300101_999AC0000000001",
                                    amendment_enforcement_date="2030-01-01", current_revision_status="Repeal"))
    cache.write(cache.revisions_path(LAW_ID), doc)
    revisions, excluded, skipped = load_revisions(cache)
    assert len(revisions) == 4 and excluded == [] and skipped == []


def test_fixture_loads_three_revisions():
    revisions, excluded, skipped = load_revisions(Cache(FIXTURE_CACHE), law_type="Act")
    assert len(revisions) == 3 and excluded == [] and skipped == []
    main, upcoming = plan(revisions)
    assert [g.commit_date for g in main] == ["2016-01-01", "2019-05-01", "2020-01-01"]
    assert upcoming == []


def test_law_with_missing_data_is_excluded(tmp_path):
    cache = Cache(tmp_path)
    cache.write(cache.revisions_path("A"), {"law_info": {"law_id": "A", "law_type": "Act", "law_num": "A号"},
                                             "revisions": [{"law_revision_id": "A_1_X", "law_type": "Act"}, {"law_revision_id": "A_2_Y", "law_type": "Act"}]})
    cache.write(cache.data_path("A_1_X"), {})
    revisions, excluded, skipped = load_revisions(cache)
    assert revisions == [] and excluded == ["A"] and skipped == []


def test_revision_recorded_as_404_is_skipped_not_excluded(tmp_path):
    cache = Cache(tmp_path)
    cache.write(cache.revisions_path("A"), {"law_info": {"law_id": "A", "law_type": "Act", "law_num": "A号"},
                                             "revisions": [{"law_revision_id": f"A_{n}_X", "law_type": "Act"} for n in (1, 2, 3)]})
    cache.write(cache.data_path("A_1_X"), {})
    cache.write(cache.data_path("A_3_X"), {})
    cache.record_failure("law_data", "A_2_X", "404 https://x/law_data/A_2_X")
    revisions, excluded, skipped = load_revisions(cache)
    assert [r.revision_id for r in revisions] == ["A_1_X", "A_3_X"]
    assert skipped == ["A_2_X"] and excluded == []


def test_not_found_reads_only_404_law_data_failures(tmp_path):
    cache = Cache(tmp_path)
    assert cache.not_found() == set()
    cache.record_failure("law_data", "A_1_X", "404 u")
    cache.record_failure("law_data", "A_2_X", "gave up after 5 attempts")
    cache.record_failure("revisions", "B", "404 u")
    assert cache.not_found() == {"A_1_X"}
