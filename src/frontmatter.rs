use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use crate::py::{json_str, truthy};

static DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap());

/// e-Gov dates an amendment whose enforcement day is not fixed yet in 2117 (12-30 or 12-31).
pub fn undetermined(date: &str) -> bool {
    date > "2100"
}

fn is_date(s: &str) -> bool {
    DATE.is_match(s)
}

fn v(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => "null".to_string(),
        Some(Value::String(s)) if is_date(s) => s.clone(),
        Some(Value::String(s)) => json_str(s),
        Some(Value::Bool(b)) => json_str(if *b { "True" } else { "False" }),
        Some(other) => json_str(&other.to_string()),
    }
}

pub fn frontmatter(law_info: &Value, rev: &Value) -> String {
    let law_id = law_info["law_id"].as_str().unwrap_or_default();
    let mut out = vec!["---".to_string()];
    for (k, val) in [
        ("law_id", Some(&law_info["law_id"])),
        ("law_num", law_info.get("law_num")),
        ("title", rev.get("law_title")),
        ("title_kana", rev.get("law_title_kana")),
        ("abbrev", rev.get("abbrev")),
        (
            "law_type",
            truthy(rev.get("law_type")).or(law_info.get("law_type")),
        ),
        ("category", rev.get("category")),
        ("revision_id", Some(&rev["law_revision_id"])),
    ] {
        out.push(format!("{k}: {}", v(val)));
    }
    out.push("amendment:".to_string());
    for (k, key) in [
        ("law_id", "amendment_law_id"),
        ("law_num", "amendment_law_num"),
        ("title", "amendment_law_title"),
        ("promulgated", "amendment_promulgate_date"),
    ] {
        out.push(format!("  {k}: {}", v(rev.get(key))));
    }
    let enforced = rev.get("amendment_enforcement_date");
    out.push(format!(
        "  enforced: {}",
        match enforced.and_then(Value::as_str) {
            Some(d) if undetermined(d) => json_str("undetermined"),
            _ => v(enforced),
        }
    ));
    // the day as the amending law words it, when it is not fixed, e.g. 政令で定める日
    if let Some(comment) = truthy(rev.get("amendment_enforcement_comment")) {
        out.push(format!("  enforcement_comment: {}", v(Some(comment))));
    }
    out.push(format!(
        "source: {}",
        json_str(&format!("https://laws.e-gov.go.jp/law/{law_id}"))
    ));
    out.push("---".to_string());
    out.push(String::new());
    out.push(String::new());
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn law_info() -> Value {
        json!({"law_id": "415AC0000000057", "law_num": "平成十五年法律第五十七号", "law_type": "Act"})
    }

    fn rev() -> Value {
        json!({
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
        })
    }

    #[test]
    fn exact() {
        assert_eq!(
            frontmatter(&law_info(), &rev()),
            "---\n\
             law_id: \"415AC0000000057\"\n\
             law_num: \"平成十五年法律第五十七号\"\n\
             title: \"個人情報の保護に関する法律\"\n\
             title_kana: \"こじんじょうほうのほごにかんするほうりつ\"\n\
             abbrev: \"個人情報保護法\"\n\
             law_type: \"Act\"\n\
             category: \"憲法\"\n\
             revision_id: \"415AC0000000057_20261001_507AC0000000070\"\n\
             amendment:\n  \
             law_id: \"507AC0000000070\"\n  \
             law_num: \"令和七年法律第七十号\"\n  \
             title: \"日本学術会議法\"\n  \
             promulgated: 2025-06-18\n  \
             enforced: 2026-10-01\n\
             source: \"https://laws.e-gov.go.jp/law/415AC0000000057\"\n\
             ---\n\n"
        );
    }

    #[test]
    fn volatile_fields_excluded_and_nulls() {
        let mut r = rev();
        r["abbrev"] = Value::Null;
        r["amendment_enforcement_date"] = Value::Null;
        let out = frontmatter(&law_info(), &r);
        assert!(out.contains("abbrev: null") && out.contains("  enforced: null"));
        assert!(!out.contains("CurrentEnforced") && !out.contains("updated"));
    }

    #[test]
    fn undetermined_enforcement_and_its_comment() {
        let mut r = rev();
        r["amendment_enforcement_date"] = json!("2117-12-31");
        r["amendment_enforcement_comment"] = json!("公布の日から起算して三年を超えない範囲内において政令で定める日");
        assert!(frontmatter(&law_info(), &r).contains(
            "  enforced: \"undetermined\"\n  \
             enforcement_comment: \"公布の日から起算して三年を超えない範囲内において政令で定める日\"\n\
             source:"
        ));
    }

    #[test]
    fn quotes_escape() {
        let mut r = rev();
        r["law_title"] = json!("a\"b:c");
        assert!(frontmatter(&law_info(), &r).contains("title: \"a\\\"b:c\""));
    }
}
