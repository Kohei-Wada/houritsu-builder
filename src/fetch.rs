use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::api::{Api, ApiError};
use crate::cache::Cache;
use crate::py::is_truthy;

const FUTURE: &str = "2099-12-31"; // asof this date, a law's revision_info is its newest revision, enforced or not

const PAGES_AT_ONCE: usize = 4; // a page takes 1.5–3 s; four at once took 2.2 s on 2026-10-17

fn law_page(client: &dyn Api, offset: usize, params: &[(&str, &str)]) -> Result<Value, ApiError> {
    let offset = offset.to_string();
    let mut all = vec![
        ("limit", "1000"),
        ("offset", offset.as_str()),
        ("response_format", "json"),
    ];
    all.extend_from_slice(params);
    client.get_json("laws", &all)
}

/// All pages of `/laws`: the first gives the total, the rest are fetched a few at a time.
pub fn fetch_law_list(client: &dyn Api, params: &[(&str, &str)]) -> Result<Vec<Value>, ApiError> {
    let first = law_page(client, 0, params)?;
    let mut laws = first["laws"].as_array().cloned().unwrap_or_default();
    let (stride, total) = (
        laws.len(),
        first["total_count"].as_u64().unwrap_or(0) as usize,
    );
    if stride == 0 || laws.len() >= total {
        return Ok(laws);
    }
    let offsets: Vec<usize> = (stride..total).step_by(stride).collect();
    for page in at_once(&offsets, PAGES_AT_ONCE, |_, &offset| {
        law_page(client, offset, params)
    }) {
        laws.extend(page?["laws"].as_array().cloned().unwrap_or_default());
    }
    Ok(laws)
}

/// Runs `f` on every item with up to `jobs` threads; the results keep the items' order.
fn at_once<T: Sync, R: Send>(
    items: &[T],
    jobs: usize,
    f: impl Fn(usize, &T) -> R + Sync,
) -> Vec<R> {
    let results: Vec<Mutex<Option<R>>> = items.iter().map(|_| Mutex::new(None)).collect();
    let next = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..jobs.max(1).min(items.len()) {
            s.spawn(|| loop {
                let k = next.fetch_add(1, Ordering::Relaxed);
                let Some(item) = items.get(k) else { return };
                *results[k].lock().unwrap() = Some(f(k, item));
            });
        }
    });
    results
        .into_iter()
        .map(|r| r.into_inner().unwrap().expect("every item ran"))
        .collect()
}

fn law_id(law: &Value) -> String {
    law["law_info"]["law_id"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

pub fn today_jst() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        + 9 * 3600;
    let z = secs.div_euclid(86400) + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn revisions_of(doc: &Value) -> &[Value] {
    doc["revisions"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn stale(
    cache: &Cache,
    law_id: &str,
    newest: [Option<&Value>; 2],
    today: &str,
    not_found: &HashSet<String>,
) -> anyhow::Result<bool> {
    let path = cache.revisions_path(law_id);
    if !path.exists() {
        return Ok(true);
    }
    let doc = cache.read(&path)?;
    let revisions = revisions_of(&doc);
    let rid = |r: &Value| {
        r["law_revision_id"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    };
    if revisions
        .iter()
        .any(|r| !not_found.contains(&rid(r)) && !cache.data_path(&rid(r)).exists())
    {
        return Ok(true); // a full text failed last time
    }
    let seen: HashSet<(String, String)> = revisions
        .iter()
        .map(|r| (rid(r), r["updated"].to_string()))
        .collect();
    if newest
        .iter()
        .flatten()
        .any(|info| is_truthy(info) && !seen.contains(&(rid(info), info["updated"].to_string())))
    {
        return Ok(true);
    }
    Ok(revisions.iter().any(|r| {
        r["current_revision_status"] == "UnEnforced"
            && r["amendment_enforcement_date"]
                .as_str()
                .filter(|d| !d.is_empty())
                .unwrap_or("9999")
                <= today
    }))
}

fn amendments(revisions: &[Value]) -> HashSet<String> {
    revisions
        .iter()
        .filter_map(|r| {
            r["amendment_law_id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
        .collect()
}

pub struct Options<'a> {
    pub law_type: Option<&'a str>,
    pub law_ids: Vec<String>,
    pub limit: Option<usize>,
    pub jobs: usize,
    pub full: bool,
}

pub type Stats = BTreeMap<&'static str, u64>;

/// The laws whose revision list to (re)fetch, never-fetched first, and every listed law's type.
fn select(
    client: &dyn Api,
    cache: &Cache,
    o: &Options,
) -> anyhow::Result<(Vec<String>, HashMap<String, String>)> {
    if !o.law_ids.is_empty() {
        return Ok((o.law_ids.clone(), HashMap::new()));
    }
    let (laws, future) = std::thread::scope(|s| {
        let future = (!o.full).then(|| s.spawn(|| fetch_law_list(client, &[("asof", FUTURE)])));
        (
            fetch_law_list(client, &[]),
            future.map(|f| f.join().unwrap()),
        )
    });
    let laws = laws?;
    cache.write(&cache.laws_path(), &Value::Array(laws.clone()))?;
    let types: HashMap<String, String> = laws
        .iter()
        .map(|l| {
            (
                law_id(l),
                l["law_info"]["law_type"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            )
        })
        .collect();
    let mut targets: Vec<String> = laws
        .iter()
        .map(law_id)
        .filter(|id| of_type(o, types.get(id)))
        .collect();
    if let Some(future) = future {
        let future: HashMap<String, Value> = future?
            .into_iter()
            .map(|l| (law_id(&l), l["revision_info"].clone()))
            .collect();
        let current: HashMap<String, Value> = laws
            .iter()
            .map(|l| (law_id(l), l["revision_info"].clone()))
            .collect();
        let (today, not_found) = (today_jst(), cache.not_found()?);
        let mut kept = vec![];
        for id in targets {
            if stale(
                cache,
                &id,
                [current.get(&id), future.get(&id)],
                &today,
                &not_found,
            )? {
                kept.push(id);
            }
        }
        targets = kept;
    }
    targets.sort_by_key(|id| cache.revisions_path(id).exists()); // never-fetched laws first
    if let Some(limit) = o.limit.filter(|l| *l > 0) {
        targets.truncate(limit);
    }
    Ok((targets, types))
}

fn of_type(o: &Options, law_type: Option<&String>) -> bool {
    o.law_type
        .is_none_or(|t| t.is_empty() || law_type.is_some_and(|x| x == t))
}

struct Run<'a> {
    client: &'a dyn Api,
    cache: &'a Cache,
    log: &'a (dyn Fn(&str) + Sync),
    stats: Mutex<Stats>,
    new_amendments: Mutex<HashSet<String>>,
}

impl Run<'_> {
    fn count(&self, key: &'static str, n: u64) {
        *self.stats.lock().unwrap().get_mut(key).unwrap() += n;
    }

    fn fail(&self, kind: &str, key: &str, e: &ApiError) -> anyhow::Result<()> {
        let mut stats = self.stats.lock().unwrap();
        self.cache.record_failure(kind, key, &e.0)?;
        *stats.get_mut("failed").unwrap() += 1;
        Ok(())
    }

    /// Refetches one law's revision list and fetches the full texts not cached yet.
    fn law(&self, i: usize, id: &str) -> anyhow::Result<()> {
        let revisions = match self.client.get_json(&format!("law_revisions/{id}"), &[]) {
            Ok(r) => r,
            Err(e) => return self.fail("law_revisions", id, &e),
        };
        let path = self.cache.revisions_path(id);
        if path.exists() {
            let old = amendments(revisions_of(&self.cache.read(&path)?));
            let added = amendments(revisions_of(&revisions))
                .into_iter()
                .filter(|a| !old.contains(a));
            self.new_amendments.lock().unwrap().extend(added);
        }
        self.cache.write(&path, &revisions)?;
        let list = revisions_of(&revisions);
        self.count("revisions", list.len() as u64);
        for rev in list {
            let rid = rev["law_revision_id"].as_str().unwrap_or_default();
            let path = self.cache.data_path(rid);
            if path.exists() {
                continue;
            }
            match self.client.get_json(
                &format!("law_data/{rid}"),
                &[("law_full_text_format", "xml"), ("response_format", "json")],
            ) {
                Ok(data) => {
                    self.cache.write(&path, &data)?;
                    self.count("fetched", 1);
                }
                Err(e) => self.fail("law_data", rid, &e)?,
            }
        }
        self.count("laws", 1);
        (self.log)(&format!("[{i}] {id} revisions={}", list.len()));
        Ok(())
    }
}

/// The laws, not yet done, that these amending laws also amend. A new amending law can add a
/// revision in the middle of a law's history, which the newest-revision check misses; e-Gov lists
/// every law it amends.
fn amended_by(
    client: &dyn Api,
    amendments: &[String],
    o: &Options,
    types: &HashMap<String, String>,
    done: &HashSet<String>,
) -> anyhow::Result<Vec<String>> {
    let mut found = vec![];
    for amendment in amendments {
        let params = [
            ("amendment_law_id", amendment.as_str()),
            ("asof", FUTURE),
            ("omit_current_revision_info", "true"),
        ];
        for l in fetch_law_list(client, &params)? {
            let id = law_id(&l);
            if !done.contains(&id) && !found.contains(&id) && of_type(o, types.get(&id)) {
                found.push(id);
            }
        }
    }
    Ok(found)
}

/// Fetches revision lists and new revisions. Unless full, only for laws whose list may have changed.
pub fn fetch(
    client: &dyn Api,
    cache: &Cache,
    o: &Options,
    log: &(dyn Fn(&str) + Sync),
) -> anyhow::Result<Stats> {
    let (mut targets, types) = select(client, cache, o)?;
    let run = Run {
        client,
        cache,
        log,
        stats: Mutex::new(BTreeMap::from([
            ("laws", 0),
            ("revisions", 0),
            ("fetched", 0),
            ("failed", 0),
        ])),
        new_amendments: Mutex::new(HashSet::new()),
    };
    let (mut done, mut queried, mut step) = (HashSet::new(), HashSet::new(), 0);
    while !targets.is_empty() {
        for result in at_once(&targets, o.jobs, |k, id| run.law(step + k + 1, id)) {
            result?;
        }
        step += targets.len();
        done.extend(targets.drain(..));
        if o.full || !o.law_ids.is_empty() {
            break;
        }
        let mut found: Vec<String> = run
            .new_amendments
            .lock()
            .unwrap()
            .drain()
            .filter(|a| queried.insert(a.clone()))
            .collect();
        found.sort();
        targets = amended_by(client, &found, o, &types, &done)?;
    }
    Ok(run.stats.into_inner().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    type Route = Box<dyn Fn(&HashMap<String, String>) -> Result<Value, ApiError> + Sync>;

    struct Fake {
        routes: HashMap<String, Route>,
        calls: Mutex<Vec<String>>,
    }

    impl Fake {
        fn new(routes: Vec<(&str, Route)>) -> Self {
            Fake {
                routes: routes
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v))
                    .collect(),
                calls: Mutex::new(vec![]),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Api for Fake {
        fn get_json(&self, path: &str, params: &[(&str, &str)]) -> Result<Value, ApiError> {
            self.calls.lock().unwrap().push(path.to_string());
            let p = params
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            (self
                .routes
                .get(path)
                .unwrap_or_else(|| panic!("no route {path}")))(&p)
        }
    }

    fn fixed(v: Value) -> Route {
        Box::new(move |_| Ok(v.clone()))
    }

    fn law(id: &str, t: &str) -> Value {
        json!({"law_info": {"law_id": id, "law_type": t}})
    }

    fn revs(id: &str, rids: &[&str]) -> Value {
        json!({"law_info": {"law_id": id}, "revisions": rids.iter().map(|r| json!({"law_revision_id": r})).collect::<Vec<_>>()})
    }

    fn opts() -> Options<'static> {
        Options {
            law_type: None,
            law_ids: vec![],
            limit: None,
            jobs: 1,
            full: false,
        }
    }

    fn quiet(_: &str) {}

    #[test]
    fn law_list_paginates() {
        let fake = Fake::new(vec![(
            "laws",
            Box::new(|p| {
                Ok(match p["offset"].as_str() {
                    "0" => json!({"total_count": 3, "laws": [law("A", "Act"), law("B", "Act")]}),
                    _ => json!({"total_count": 3, "laws": [law("C", "Act")]}),
                })
            }),
        )]);
        let ids: Vec<String> = fetch_law_list(&fake, &[])
            .unwrap()
            .iter()
            .map(law_id)
            .collect();
        assert_eq!(ids, ["A", "B", "C"]);
    }

    #[test]
    fn law_list_pages_fetched_at_once_keep_their_order() {
        let fake = Fake::new(vec![(
            "laws",
            Box::new(|p| {
                let offset: usize = p["offset"].parse().unwrap();
                let ids: Vec<Value> = (offset..(offset + 2).min(11))
                    .map(|n| law(&format!("L{n:02}"), "Act"))
                    .collect();
                Ok(json!({"total_count": 11, "laws": ids}))
            }),
        )]);
        let ids: Vec<String> = fetch_law_list(&fake, &[])
            .unwrap()
            .iter()
            .map(law_id)
            .collect();
        assert_eq!(ids, (0..11).map(|n| format!("L{n:02}")).collect::<Vec<_>>());
        assert_eq!(fake.calls().len(), 6);
    }

    #[test]
    fn fetch_filters_skips_cached_and_records_failures() {
        let d = tempfile::tempdir().unwrap();
        let cache = Cache::new(d.path());
        cache
            .write(&cache.data_path("A_1_X"), &json!({"cached": true}))
            .unwrap();
        let fake = Fake::new(vec![
            (
                "laws",
                fixed(
                    json!({"total_count": 3, "laws": [law("A", "Act"), law("B", "Act"), law("O", "CabinetOrder")]}),
                ),
            ),
            ("law_revisions/A", fixed(revs("A", &["A_1_X", "A_2_Y"]))),
            (
                "law_revisions/B",
                Box::new(|_| Err(ApiError("gave up".into()))),
            ),
            ("law_data/A_2_Y", fixed(json!({"law_full_text": "x"}))),
        ]);
        let stats = fetch(
            &fake,
            &cache,
            &Options {
                law_type: Some("Act"),
                ..opts()
            },
            &quiet,
        )
        .unwrap();
        assert!(
            !fake.calls().contains(&"law_data/A_1_X".to_string())
                && !fake.calls().contains(&"law_revisions/O".to_string())
        );
        assert_eq!(
            cache.read(&cache.data_path("A_2_Y")).unwrap(),
            json!({"law_full_text": "x"})
        );
        assert_eq!(
            stats,
            BTreeMap::from([("laws", 1), ("revisions", 2), ("fetched", 1), ("failed", 1)])
        );
        assert!(std::fs::read_to_string(d.path().join("failures.jsonl"))
            .unwrap()
            .contains("law_revisions"));
    }

    #[test]
    fn fetch_by_law_id_skips_listing() {
        let d = tempfile::tempdir().unwrap();
        let fake = Fake::new(vec![("law_revisions/A", fixed(revs("A", &[])))]);
        fetch(
            &fake,
            &Cache::new(d.path()),
            &Options {
                law_ids: vec!["A".into()],
                ..opts()
            },
            &quiet,
        )
        .unwrap();
        assert_eq!(fake.calls(), ["law_revisions/A"]);
    }

    #[test]
    fn fetch_takes_never_fetched_laws_first() {
        let d = tempfile::tempdir().unwrap();
        let cache = Cache::new(d.path());
        cache
            .write(&cache.revisions_path("A"), &revs("A", &[]))
            .unwrap();
        let fake = Fake::new(vec![
            (
                "laws",
                fixed(json!({"total_count": 2, "laws": [law("A", "Act"), law("B", "Act")]})),
            ),
            ("law_revisions/A", fixed(revs("A", &[]))),
            ("law_revisions/B", fixed(revs("B", &[]))),
        ]);
        fetch(
            &fake,
            &cache,
            &Options {
                full: true,
                ..opts()
            },
            &quiet,
        )
        .unwrap();
        assert_eq!(fake.calls(), ["laws", "law_revisions/B", "law_revisions/A"]);
    }

    #[test]
    fn fetch_in_parallel_gets_everything() {
        let d = tempfile::tempdir().unwrap();
        let cache = Cache::new(d.path());
        let ids: Vec<String> = (0..20).map(|n| format!("L{n}")).collect();
        let mut routes: Vec<(String, Route)> = vec![(
            "laws".into(),
            fixed(
                json!({"total_count": 20, "laws": ids.iter().map(|i| law(i, "Act")).collect::<Vec<_>>()}),
            ),
        )];
        for i in &ids {
            let route: Route = if i == "L3" {
                Box::new(|_| Err(ApiError("gave up".into())))
            } else {
                fixed(revs(i, &[&format!("{i}_1_X"), &format!("{i}_2_Y")]))
            };
            routes.push((format!("law_revisions/{i}"), route));
            for s in ["1_X", "2_Y"] {
                routes.push((
                    format!("law_data/{i}_{s}"),
                    fixed(json!({"law_full_text": i})),
                ));
            }
        }
        let fake = Fake {
            routes: routes.into_iter().collect(),
            calls: Mutex::new(vec![]),
        };
        let stats = fetch(&fake, &cache, &Options { jobs: 4, ..opts() }, &quiet).unwrap();
        assert_eq!(
            stats,
            BTreeMap::from([
                ("laws", 19),
                ("revisions", 38),
                ("fetched", 38),
                ("failed", 1)
            ])
        );
        assert_eq!(
            cache.read(&cache.data_path("L7_2_Y")).unwrap(),
            json!({"law_full_text": "L7"})
        );
    }

    fn listed(id: &str, rid: &str, updated: &str) -> Value {
        json!({"law_info": {"law_id": id, "law_type": "Act"}, "revision_info": {"law_revision_id": rid, "updated": updated}})
    }

    fn rev(rid: &str, amendment: Option<&str>, status: &str, enforced: &str) -> Value {
        json!({"law_revision_id": rid, "amendment_law_id": amendment, "current_revision_status": status,
               "amendment_enforcement_date": enforced, "updated": "u1"})
    }

    fn law_lists(
        now: Vec<Value>,
        future: Vec<Value>,
        by_amendment: HashMap<String, Vec<Value>>,
    ) -> Route {
        Box::new(move |p| {
            let found = match (p.get("amendment_law_id"), p.get("asof")) {
                (Some(a), _) => by_amendment.get(a).cloned().unwrap_or_default(),
                (None, Some(_)) => future.clone(),
                (None, None) => now.clone(),
            };
            Ok(json!({"total_count": found.len(), "laws": found}))
        })
    }

    fn doc(id: &str, revisions: Vec<Value>) -> Value {
        json!({"law_info": {"law_id": id}, "revisions": revisions})
    }

    #[test]
    fn delta_refetches_only_laws_that_may_have_changed() {
        let d = tempfile::tempdir().unwrap();
        let cache = Cache::new(d.path());
        let cached = [
            (
                "SAME",
                vec![rev("SAME_1", None, "CurrentEnforced", "2020-01-01")],
            ),
            (
                "NEWER",
                vec![rev("NEWER_1", None, "CurrentEnforced", "2020-01-01")],
            ),
            (
                "EDITED",
                vec![rev("EDITED_1", None, "CurrentEnforced", "2020-01-01")],
            ),
            (
                "DUE",
                vec![
                    rev("DUE_2", None, "UnEnforced", "2000-01-01"),
                    rev("DUE_1", None, "CurrentEnforced", "2020-01-01"),
                ],
            ),
            (
                "LATER",
                vec![
                    rev("LATER_2", None, "UnEnforced", "9999-01-01"),
                    rev("LATER_1", None, "CurrentEnforced", "2020-01-01"),
                ],
            ),
            (
                "FAILED",
                vec![rev("FAILED_1", None, "CurrentEnforced", "2020-01-01")],
            ),
            (
                "GONE",
                vec![rev("GONE_1", None, "CurrentEnforced", "2020-01-01")],
            ),
        ];
        for (id, revisions) in &cached {
            cache
                .write(&cache.revisions_path(id), &doc(id, revisions.clone()))
                .unwrap();
            if !["FAILED", "GONE"].contains(id) {
                for r in revisions {
                    cache
                        .write(
                            &cache.data_path(r["law_revision_id"].as_str().unwrap()),
                            &json!({"law_full_text": ""}),
                        )
                        .unwrap();
                }
            }
        }
        cache
            .record_failure("law_data", "GONE_1", "404 https://x")
            .unwrap();
        let now = vec![
            listed("SAME", "SAME_1", "u1"),
            listed("NEWER", "NEWER_1", "u1"),
            listed("EDITED", "EDITED_1", "u2"),
            listed("DUE", "DUE_1", "u1"),
            listed("LATER", "LATER_1", "u1"),
            listed("NEW", "NEW_1", "u1"),
            listed("FAILED", "FAILED_1", "u1"),
            listed("GONE", "GONE_1", "u1"),
        ];
        let future = vec![
            listed("SAME", "SAME_1", "u1"),
            listed("NEWER", "NEWER_2", "u1"),
            listed("EDITED", "EDITED_1", "u2"),
            listed("DUE", "DUE_2", "u1"),
            listed("LATER", "LATER_2", "u1"),
            listed("NEW", "NEW_1", "u1"),
            listed("FAILED", "FAILED_1", "u1"),
            listed("GONE", "GONE_1", "u1"),
        ];
        let mut routes: Vec<(&str, Route)> = vec![("laws", law_lists(now, future, HashMap::new()))];
        let paths: Vec<String> = ["NEWER", "EDITED", "DUE", "NEW", "FAILED"]
            .iter()
            .map(|id| format!("law_revisions/{id}"))
            .collect();
        for (p, id) in paths
            .iter()
            .zip(["NEWER", "EDITED", "DUE", "NEW", "FAILED"])
        {
            routes.push((p.as_str(), fixed(doc(id, vec![]))));
        }
        let fake = Fake::new(routes);
        fetch(&fake, &cache, &opts(), &quiet).unwrap();
        let mut got: Vec<String> = fake.calls().into_iter().filter(|c| c != "laws").collect();
        got.sort();
        assert_eq!(
            got,
            [
                "law_revisions/DUE",
                "law_revisions/EDITED",
                "law_revisions/FAILED",
                "law_revisions/NEW",
                "law_revisions/NEWER"
            ]
        );
    }

    #[test]
    fn delta_follows_a_new_amending_law_to_the_other_laws_it_amends() {
        let d = tempfile::tempdir().unwrap();
        let cache = Cache::new(d.path());
        cache
            .write(
                &cache.revisions_path("A"),
                &doc(
                    "A",
                    vec![rev("A_1", Some("X1"), "CurrentEnforced", "2020-01-01")],
                ),
            )
            .unwrap();
        cache
            .write(
                &cache.revisions_path("B"),
                &doc(
                    "B",
                    vec![
                        rev("B_3", Some("X3"), "CurrentEnforced", "2020-01-01"),
                        rev("B_1", Some("X1"), "CurrentEnforced", "2020-01-01"),
                    ],
                ),
            )
            .unwrap();
        for rid in ["A_1", "B_1", "B_3"] {
            cache
                .write(&cache.data_path(rid), &json!({"law_full_text": ""}))
                .unwrap();
        }
        let now = vec![listed("A", "A_1", "u1"), listed("B", "B_1", "u1")];
        let future = vec![listed("A", "A_2", "u1"), listed("B", "B_3", "u1")];
        let by = HashMap::from([(
            "X2".to_string(),
            vec![listed("A", "A_2", "u1"), listed("B", "B_3", "u1")],
        )]);
        let fake = Fake::new(vec![
            ("laws", law_lists(now, future, by)),
            (
                "law_revisions/A",
                fixed(doc(
                    "A",
                    vec![
                        rev("A_2", Some("X2"), "CurrentEnforced", "2020-01-01"),
                        rev("A_1", Some("X1"), "CurrentEnforced", "2020-01-01"),
                    ],
                )),
            ),
            (
                "law_revisions/B",
                fixed(doc(
                    "B",
                    vec![
                        rev("B_3", Some("X3"), "CurrentEnforced", "2020-01-01"),
                        rev("B_2", Some("X2"), "CurrentEnforced", "2020-01-01"),
                        rev("B_1", Some("X1"), "CurrentEnforced", "2020-01-01"),
                    ],
                )),
            ),
            ("law_data/A_2", fixed(json!({"law_full_text": "a"}))),
            ("law_data/B_2", fixed(json!({"law_full_text": "b"}))),
        ]);
        fetch(&fake, &cache, &opts(), &quiet).unwrap();
        assert_eq!(
            cache.read(&cache.data_path("B_2")).unwrap(),
            json!({"law_full_text": "b"})
        );
        let calls = fake.calls();
        assert_eq!(calls.iter().filter(|c| *c == "law_revisions/A").count(), 1);
        assert_eq!(calls.iter().filter(|c| *c == "law_revisions/B").count(), 1);
    }
}
