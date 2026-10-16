import json
import re

_DATE = re.compile(r"\d{4}-\d{2}-\d{2}")


def _v(value) -> str:
    if value is None:
        return "null"
    if isinstance(value, str) and _DATE.fullmatch(value):
        return value
    return json.dumps(str(value), ensure_ascii=False)


def frontmatter(law_info: dict, rev: dict) -> str:
    law_id = law_info["law_id"]
    lines = [
        ("law_id", law_id),
        ("law_num", law_info.get("law_num")),
        ("title", rev.get("law_title")),
        ("title_kana", rev.get("law_title_kana")),
        ("abbrev", rev.get("abbrev")),
        ("law_type", rev.get("law_type") or law_info.get("law_type")),
        ("category", rev.get("category")),
        ("revision_id", rev["law_revision_id"]),
    ]
    out = ["---"] + [f"{k}: {_v(v)}" for k, v in lines]
    out += ["amendment:"] + [
        f"  {k}: {_v(v)}"
        for k, v in (
            ("law_id", rev.get("amendment_law_id")),
            ("law_num", rev.get("amendment_law_num")),
            ("title", rev.get("amendment_law_title")),
            ("promulgated", rev.get("amendment_promulgate_date")),
            ("enforced", rev.get("amendment_enforcement_date")),
        )
    ]
    out += [
        f"repeal_status: {_v(rev.get('repeal_status'))}",
        f"source: {_v(f'https://laws.e-gov.go.jp/law/{law_id}')}",
        "---",
        "",
        "",
    ]
    return "\n".join(out)
