use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::Value;

use crate::cache::Cache;
use crate::py::{or_str, truthy};

pub fn kebab(law_type: &str) -> String {
    let mut out = String::new();
    for (i, c) in law_type.chars().enumerate() {
        if i > 0 && c.is_ascii_uppercase() {
            out.push('-');
        }
        out.extend(c.to_lowercase());
    }
    out
}

fn as_str(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str)
}

#[derive(Clone, Debug)]
pub struct Revision {
    pub law_id: String,
    pub law_num: String,
    pub law_type: String,
    pub law_info: Arc<Value>,
    pub rev: Value,
}

impl Revision {
    pub fn revision_id(&self) -> &str {
        self.rev["law_revision_id"].as_str().unwrap_or_default()
    }

    pub fn title(&self) -> &str {
        as_str(truthy(self.rev.get("law_title"))).unwrap_or(&self.law_id)
    }

    pub fn amendment_law_id(&self) -> &str {
        as_str(truthy(self.rev.get("amendment_law_id"))).unwrap_or(&self.law_id)
    }

    pub fn enforced(&self) -> Option<&str> {
        as_str(self.rev.get("amendment_enforcement_date"))
    }

    pub fn promulgated(&self) -> Option<&str> {
        as_str(self.rev.get("amendment_promulgate_date"))
    }

    pub fn upcoming(&self) -> bool {
        self.rev.get("current_revision_status").and_then(Value::as_str) == Some("UnEnforced")
    }

    pub fn repealed(&self) -> bool {
        self.rev.get("current_revision_status").and_then(Value::as_str) == Some("Repeal")
    }

    pub fn path(&self) -> String {
        let first = self.law_type.split(',').next().unwrap_or_default();
        let shard: String = self.law_id.chars().take(3).collect();
        format!("laws/{}/{shard}/{}.md", kebab(first), self.law_id)
    }
}

pub struct Group {
    pub amendment_law_id: String,
    pub enforced: Option<String>,
    pub revisions: Vec<Revision>,
}

impl Group {
    pub fn upcoming(&self) -> bool {
        self.revisions.iter().any(Revision::upcoming)
    }

    pub fn commit_date(&self) -> String {
        or_str(self.enforced.as_deref(), or_str(self.revisions[0].promulgated(), "1970-01-02")).to_string()
    }

    pub fn author(&self) -> String {
        let first = &self.revisions[0];
        match as_str(truthy(first.rev.get("amendment_law_num"))) {
            Some(num) => num.to_string(),
            None if first.amendment_law_id() == first.law_id => first.law_num.clone(),
            None => first.amendment_law_id().to_string(),
        }
    }

    pub fn message(&self) -> String {
        let first = &self.revisions[0];
        let rev = &first.rev;
        let title = or_str(as_str(rev.get("amendment_law_title")), first.title());
        let mut lines = vec![
            format!("{title} 施行"),
            String::new(),
            format!("改正法令: {}", or_str(as_str(rev.get("amendment_law_num")), "不明")),
            format!("公布日: {}", or_str(as_str(rev.get("amendment_promulgate_date")), "不明")),
            format!("施行日: {}", or_str(self.enforced.as_deref(), "未定")),
            String::new(),
            "変更された法令:".to_string(),
        ];
        for r in &self.revisions {
            lines.push(format!("- {}（{}）{}", r.title(), r.law_num, if r.repealed() { "（廃止）" } else { "" }));
        }
        lines.push(String::new());
        lines.push(format!("Amendment-Law-Id: {}", self.amendment_law_id));
        lines.push(format!("Enforced: {}", or_str(self.enforced.as_deref(), "unknown")));
        lines.join("\n") + "\n"
    }
}

pub struct Loaded {
    pub revisions: Vec<Revision>,
    pub excluded: Vec<String>,
    pub skipped: Vec<String>,
}

pub fn load_revisions(cache: &Cache, law_type: Option<&str>) -> anyhow::Result<Loaded> {
    let mut loaded = Loaded { revisions: vec![], excluded: vec![], skipped: vec![] };
    let not_found = cache.not_found()?;
    for path in cache.revision_files()? {
        let mut doc = cache.read(&path)?;
        let info = Arc::new(doc["law_info"].take());
        if let Some(t) = law_type.filter(|t| !t.is_empty()) {
            if as_str(info.get("law_type")) != Some(t) {
                continue;
            }
        }
        let law_id = info["law_id"].as_str().unwrap_or_default().to_string();
        let law_num = or_str(as_str(info.get("law_num")), "").to_string();
        let revs: Vec<Revision> = match doc["revisions"].take() {
            Value::Array(a) => a,
            _ => vec![],
        }
        .into_iter()
        .map(|r| Revision {
            law_id: law_id.clone(),
            law_num: law_num.clone(),
            law_type: as_str(truthy(r.get("law_type")).or(info.get("law_type"))).unwrap_or_default().to_string(),
            law_info: info.clone(),
            rev: r,
        })
        .collect();
        if revs.is_empty() {
            continue;
        }
        let missing: HashSet<String> = revs
            .iter()
            .filter(|r| !r.repealed() && !cache.data_path(r.revision_id()).exists())
            .map(|r| r.revision_id().to_string())
            .collect();
        if missing.iter().any(|m| !not_found.contains(m)) {
            loaded.excluded.push(law_id);
            continue;
        }
        loaded.skipped.extend(revs.iter().filter(|r| missing.contains(r.revision_id())).map(|r| r.revision_id().to_string()));
        loaded.revisions.extend(revs.into_iter().filter(|r| !missing.contains(r.revision_id())));
    }
    Ok(loaded)
}

fn drop_redundant_repeals(mut revisions: Vec<Revision>) -> Vec<Revision> {
    revisions.sort_by(|a, b| {
        let key = |r: &Revision| (or_str(r.enforced(), or_str(r.promulgated(), "")).to_string(), r.amendment_law_id().to_string());
        key(a).cmp(&key(b))
    });
    let mut alive = HashSet::new();
    let mut kept = vec![];
    for r in revisions {
        if r.repealed() {
            if !alive.remove(&r.law_id) {
                continue;
            }
        } else {
            alive.insert(r.law_id.clone());
        }
        kept.push(r);
    }
    kept
}

pub fn plan(revisions: Vec<Revision>) -> (Vec<Group>, Vec<Group>) {
    let mut groups: Vec<Group> = vec![];
    let mut index: HashMap<(Option<String>, String), usize> = HashMap::new();
    for r in drop_redundant_repeals(revisions) {
        let key = (r.enforced().map(str::to_string), r.amendment_law_id().to_string());
        let i = *index.entry(key).or_insert_with(|| {
            groups.push(Group { amendment_law_id: r.amendment_law_id().to_string(), enforced: r.enforced().map(str::to_string), revisions: vec![] });
            groups.len() - 1
        });
        groups[i].revisions.push(r);
    }
    for g in &mut groups {
        g.revisions.sort_by_cached_key(Revision::path);
    }
    groups.sort_by_cached_key(|g| (g.commit_date(), g.amendment_law_id.clone()));
    groups.into_iter().partition(|g| !g.upcoming())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{fixture_cache, LAW_ID};
    use serde_json::json;

    fn revision(law_id: &str, law_num: &str, law_type: &str, rev: Value) -> Revision {
        Revision {
            law_id: law_id.into(),
            law_num: law_num.into(),
            law_type: law_type.into(),
            law_info: Arc::new(json!({"law_id": law_id, "law_num": law_num})),
            rev,
        }
    }

    fn rev(law_id: &str, amend: &str, enforced: Option<&str>, promulgated: &str, status: &str, title: &str) -> Revision {
        let rid = format!("{law_id}_{}_{amend}", enforced.unwrap_or("x").replace('-', ""));
        revision(
            law_id,
            &format!("{law_id}号"),
            "Act",
            json!({
                "law_revision_id": rid, "law_title": title, "amendment_law_id": amend, "amendment_law_num": format!("{amend}号"),
                "amendment_law_title": "改正法", "amendment_enforcement_date": enforced,
                "amendment_promulgate_date": promulgated, "current_revision_status": status,
            }),
        )
    }

    fn r(law_id: &str, amend: &str, enforced: &str) -> Revision {
        rev(law_id, amend, Some(enforced), "2020-01-01", "PreviousEnforced", "法")
    }

    fn ids(groups: &[Group]) -> Vec<(&str, Vec<&str>)> {
        groups.iter().map(|g| (g.amendment_law_id.as_str(), g.revisions.iter().map(|r| r.law_id.as_str()).collect())).collect()
    }

    #[test]
    fn kebab_cases() {
        let got: Vec<String> = ["Act", "CabinetOrder", "MinisterialOrdinance"].iter().map(|t| kebab(t)).collect();
        assert_eq!(got, ["act", "cabinet-order", "ministerial-ordinance"]);
    }

    #[test]
    fn path_shards_by_era_year_and_uses_first_law_type() {
        assert_eq!(revision("415AC0000000057", "", "Act", json!({})).path(), "laws/act/415/415AC0000000057.md");
        assert_eq!(revision("322AC0000000067", "", "Act,CabinetOrder", json!({})).path(), "laws/act/322/322AC0000000067.md");
    }

    #[test]
    fn one_amending_law_changing_two_laws_is_one_commit() {
        let (main, upcoming) = plan(vec![r("B", "Z", "2021-04-01"), r("A", "Z", "2021-04-01"), r("A", "Y", "2020-04-01")]);
        assert_eq!(ids(&main), [("Y", vec!["A"]), ("Z", vec!["A", "B"])]);
        assert!(upcoming.is_empty());
    }

    #[test]
    fn same_day_revisions_are_ordered_by_amendment_id() {
        let (main, _) = plan(vec![r("A", "Q2", "2026-07-17"), r("A", "Q1", "2026-07-17")]);
        assert_eq!(main.iter().map(|g| g.amendment_law_id.as_str()).collect::<Vec<_>>(), ["Q1", "Q2"]);
    }

    #[test]
    fn unknown_enforcement_date_goes_upcoming() {
        let (main, upcoming) = plan(vec![r("A", "Y", "2020-04-01"), rev("A", "Z", None, "2026-06-01", "UnEnforced", "法")]);
        assert_eq!(main.iter().map(|g| g.amendment_law_id.as_str()).collect::<Vec<_>>(), ["Y"]);
        assert_eq!(upcoming[0].commit_date(), "2026-06-01");
        assert!(upcoming[0].message().ends_with("Amendment-Law-Id: Z\nEnforced: unknown\n"));
    }

    #[test]
    fn author_is_the_amending_law_number() {
        assert_eq!(plan(vec![r("A", "Z", "2021-04-01")]).0[0].author(), "Z号");
        let first = revision("A", "平成元年法律第一号", "Act", json!({"law_revision_id": "A_x", "amendment_enforcement_date": "2020-01-01"}));
        assert_eq!(plan(vec![first]).0[0].author(), "平成元年法律第一号");
    }

    #[test]
    fn message_format() {
        let (main, _) = plan(vec![
            rev("B", "Z", Some("2027-01-17"), "2026-07-17", "PreviousEnforced", "乙法"),
            rev("A", "Z", Some("2027-01-17"), "2026-07-17", "PreviousEnforced", "甲法"),
        ]);
        assert_eq!(
            main[0].message(),
            "改正法 施行\n\n改正法令: Z号\n公布日: 2026-07-17\n施行日: 2027-01-17\n\n変更された法令:\n- 甲法（A号）\n- 乙法（B号）\n\n\
             Amendment-Law-Id: Z\nEnforced: 2027-01-17\n"
        );
    }

    #[test]
    fn repeal_is_kept_once_and_marked_in_message() {
        let (main, _) = plan(vec![
            r("A", "Y", "2020-04-01"),
            rev("A", "Z", Some("2021-04-01"), "2020-01-01", "Repeal", "法"),
            rev("A", "W", Some("2022-04-01"), "2020-01-01", "Repeal", "法"),
            rev("B", "Z", Some("2021-04-01"), "2020-01-01", "PreviousEnforced", "乙法"),
        ]);
        let got: Vec<_> =
            main.iter().map(|g| (g.amendment_law_id.as_str(), g.revisions.iter().map(|r| (r.law_id.as_str(), r.repealed())).collect::<Vec<_>>())).collect();
        assert_eq!(got, [("Y", vec![("A", false)]), ("Z", vec![("A", true), ("B", false)])]);
        assert!(main[1].message().contains("- 法（A号）（廃止）\n- 乙法（B号）\n"));
    }

    #[test]
    fn repeal_of_law_never_seen_is_dropped() {
        assert!(plan(vec![rev("A", "Z", Some("2021-04-01"), "2020-01-01", "Repeal", "法")]).0.is_empty());
    }

    #[test]
    fn repealed_revision_needs_no_text() {
        let (_d, cache) = fixture_cache();
        let mut doc = cache.read(&cache.revisions_path(LAW_ID)).unwrap();
        let mut repeal = doc["revisions"][0].clone();
        repeal["law_revision_id"] = json!(format!("{LAW_ID}_20300101_999AC0000000001"));
        repeal["amendment_enforcement_date"] = json!("2030-01-01");
        repeal["current_revision_status"] = json!("Repeal");
        doc["revisions"].as_array_mut().unwrap().insert(0, repeal);
        cache.write(&cache.revisions_path(LAW_ID), &doc).unwrap();
        let l = load_revisions(&cache, None).unwrap();
        assert!(l.revisions.len() == 4 && l.excluded.is_empty() && l.skipped.is_empty());
    }

    #[test]
    fn fixture_loads_three_revisions() {
        let (_d, cache) = fixture_cache();
        let l = load_revisions(&cache, Some("Act")).unwrap();
        assert!(l.revisions.len() == 3 && l.excluded.is_empty() && l.skipped.is_empty());
        let (main, upcoming) = plan(l.revisions);
        assert_eq!(main.iter().map(Group::commit_date).collect::<Vec<_>>(), ["2016-01-01", "2019-05-01", "2020-01-01"]);
        assert!(upcoming.is_empty());
    }

    #[test]
    fn law_with_missing_data_is_excluded() {
        let d = tempfile::tempdir().unwrap();
        let cache = Cache::new(d.path());
        cache
            .write(
                &cache.revisions_path("A"),
                &json!({"law_info": {"law_id": "A", "law_type": "Act", "law_num": "A号"},
                        "revisions": [{"law_revision_id": "A_1_X", "law_type": "Act"}, {"law_revision_id": "A_2_Y", "law_type": "Act"}]}),
            )
            .unwrap();
        cache.write(&cache.data_path("A_1_X"), &json!({})).unwrap();
        let l = load_revisions(&cache, None).unwrap();
        assert!(l.revisions.is_empty() && l.excluded == ["A"] && l.skipped.is_empty());
    }

    #[test]
    fn revision_recorded_as_404_is_skipped_not_excluded() {
        let d = tempfile::tempdir().unwrap();
        let cache = Cache::new(d.path());
        let revs: Vec<Value> = (1..=3).map(|n| json!({"law_revision_id": format!("A_{n}_X"), "law_type": "Act"})).collect();
        cache.write(&cache.revisions_path("A"), &json!({"law_info": {"law_id": "A", "law_type": "Act", "law_num": "A号"}, "revisions": revs})).unwrap();
        cache.write(&cache.data_path("A_1_X"), &json!({})).unwrap();
        cache.write(&cache.data_path("A_3_X"), &json!({})).unwrap();
        cache.record_failure("law_data", "A_2_X", "404 https://x/law_data/A_2_X").unwrap();
        let l = load_revisions(&cache, None).unwrap();
        assert_eq!(l.revisions.iter().map(Revision::revision_id).collect::<Vec<_>>(), ["A_1_X", "A_3_X"]);
        assert!(l.skipped == ["A_2_X"] && l.excluded.is_empty());
    }
}
