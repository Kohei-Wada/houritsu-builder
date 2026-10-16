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


def test_fixture_loads_three_revisions():
    revisions, excluded = load_revisions(Cache(FIXTURE_CACHE), law_type="Act")
    assert len(revisions) == 3 and excluded == []
    main, upcoming = plan(revisions)
    assert [g.commit_date for g in main] == ["2016-01-01", "2019-05-01", "2020-01-01"]
    assert upcoming == []


def test_law_with_missing_data_is_excluded(tmp_path):
    cache = Cache(tmp_path)
    cache.write(cache.revisions_path("A"), {"law_info": {"law_id": "A", "law_type": "Act", "law_num": "A号"},
                                             "revisions": [{"law_revision_id": "A_1_X", "law_type": "Act"}, {"law_revision_id": "A_2_Y", "law_type": "Act"}]})
    cache.write(cache.data_path("A_1_X"), {})
    revisions, excluded = load_revisions(cache)
    assert revisions == [] and excluded == ["A"]
