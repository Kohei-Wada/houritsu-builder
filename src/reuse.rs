//! Reusing the previous build's blobs: `rendered.json` in the cache maps each revision to the blob
//! it rendered to, valid while the revision's input key and the code version are unchanged.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;
use std::sync::OnceLock;
use std::time::UNIX_EPOCH;

use serde_json::{json, Value};
use sha1::{Digest, Sha1};

use crate::cache::Cache;
use crate::fastimport::hex;
use crate::history::Revision;
use crate::render::Counts;

const FILE: &str = "rendered.json";

/// Input key, blob sha and the unknown elements counted while rendering.
pub type Entry = (String, String, Counts);
pub type Entries = BTreeMap<String, Entry>;

/// Everything a blob or its lint result depends on. Fetch or CLI changes keep the previous renders.
const RENDER_SOURCES: [&str; 9] = [
    include_str!("render.rs"),
    include_str!("fidelity.rs"),
    include_str!("frontmatter.rs"),
    include_str!("xmltext.rs"),
    include_str!("py.rs"),
    include_str!("build.rs"),
    include_str!("lint.rs"),
    include_str!("../assets/rumdl.toml"),
    include_str!("../Cargo.lock"),
];

pub fn code_version() -> &'static str {
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION.get_or_init(|| {
        let mut h = Sha1::new();
        for source in RENDER_SOURCES {
            h.update(source);
            h.update([0]);
        }
        hex(&h.finalize())
    })
}

fn canonical(v: &Value) -> String {
    match v {
        Value::Object(m) => {
            let sorted: BTreeMap<&String, String> = m.iter().map(|(k, v)| (k, canonical(v))).collect();
            let fields: Vec<String> = sorted.into_iter().map(|(k, v)| format!("{}:{v}", Value::String(k.clone()))).collect();
            format!("{{{}}}", fields.join(","))
        }
        Value::Array(a) => format!("[{}]", a.iter().map(canonical).collect::<Vec<_>>().join(",")),
        other => other.to_string(),
    }
}

/// Changes when the revision's data file is rewritten or its metadata changes.
pub fn input_key(cache: &Cache, r: &Revision) -> anyhow::Result<String> {
    let st = fs::metadata(cache.data_path(r.revision_id()))?;
    let mtime = st.modified()?.duration_since(UNIX_EPOCH)?.as_nanos();
    let meta = canonical(&json!([&*r.law_info, &r.rev]));
    Ok(hex(&Sha1::digest(format!("{} {mtime} {meta}", st.len()))))
}

/// The entries of the last linted build whose blobs `repo` still has.
pub fn load(cache: &Cache, repo: &Path, version: &str) -> anyhow::Result<HashMap<String, Entry>> {
    let path = cache.root.join(FILE);
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let doc: Value = serde_json::from_str(&fs::read_to_string(&path)?)?;
    if doc["version"] != version {
        return Ok(HashMap::new());
    }
    let entries: HashMap<String, Entry> = serde_json::from_value(doc["entries"].clone())?;
    let present = crate::git::present(repo, entries.values().map(|e| e.1.as_str()))?;
    Ok(entries.into_iter().filter(|(_, e)| present.contains(&e.1)).collect())
}

pub fn save(cache: &Cache, version: &str, entries: &Entries) -> anyhow::Result<()> {
    fs::write(cache.root.join(FILE), serde_json::to_string(&json!({"version": version, "entries": entries}))?)?;
    Ok(())
}
