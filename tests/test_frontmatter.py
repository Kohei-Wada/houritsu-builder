from houritsu_builder.frontmatter import frontmatter

LAW_INFO = {"law_id": "415AC0000000057", "law_num": "平成十五年法律第五十七号", "law_type": "Act"}
REV = {
    "law_revision_id": "415AC0000000057_20261001_507AC0000000070",
    "law_title": "個人情報の保護に関する法律",
    "law_title_kana": "こじんじょうほうのほごにかんするほうりつ",
    "abbrev": "個人情報保護法",
    "law_type": "Act",
    "category": "憲法",
    "amendment_law_id": "507AC0000000070",
    "amendment_law_num": "令和七年法律第七十号",
    "amendment_law_title": "日本学術会議法",
    "amendment_promulgate_date": "2025-06-18",
    "amendment_enforcement_date": "2026-10-01",
    "repeal_status": "None",
    "current_revision_status": "CurrentEnforced",
    "updated": "2026-07-24T11:46:53+09:00",
}


def test_frontmatter_exact():
    assert frontmatter(LAW_INFO, REV) == (
        "---\n"
        'law_id: "415AC0000000057"\n'
        'law_num: "平成十五年法律第五十七号"\n'
        'title: "個人情報の保護に関する法律"\n'
        'title_kana: "こじんじょうほうのほごにかんするほうりつ"\n'
        'abbrev: "個人情報保護法"\n'
        'law_type: "Act"\n'
        'category: "憲法"\n'
        'revision_id: "415AC0000000057_20261001_507AC0000000070"\n'
        "amendment:\n"
        '  law_id: "507AC0000000070"\n'
        '  law_num: "令和七年法律第七十号"\n'
        '  title: "日本学術会議法"\n'
        "  promulgated: 2025-06-18\n"
        "  enforced: 2026-10-01\n"
        'repeal_status: "None"\n'
        'source: "https://laws.e-gov.go.jp/law/415AC0000000057"\n'
        "---\n\n"
    )


def test_volatile_fields_excluded_and_nulls():
    out = frontmatter(LAW_INFO, {**REV, "abbrev": None, "amendment_enforcement_date": None})
    assert "abbrev: null" in out and "  enforced: null" in out
    assert "CurrentEnforced" not in out and "updated" not in out


def test_quotes_escape():
    out = frontmatter(LAW_INFO, {**REV, "law_title": 'a"b:c'})
    assert 'title: "a\\"b:c"' in out
