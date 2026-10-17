use std::path::{Path, PathBuf};

use base64::Engine;

use crate::cache::Cache;

pub const LAW_ID: &str = "323AC1000000178";
pub const SUFFIXES: [&str; 3] = [
    "20160101_426AC0000000043",
    "20190501_429AC0000000063",
    "20200101_430AC1000000057",
];

pub fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn decode(cache: &Cache, path: &Path) -> String {
    let data = cache.read(path).unwrap();
    String::from_utf8(
        base64::engine::general_purpose::STANDARD
            .decode(data["law_full_text"].as_str().unwrap())
            .unwrap(),
    )
    .unwrap()
}

pub fn fixture_xml(suffix: &str) -> String {
    let cache = Cache::new(fixtures().join("cache"));
    decode(&cache, &cache.data_path(&format!("{LAW_ID}_{suffix}")))
}

pub fn fixture_xmls() -> Vec<String> {
    let cache = Cache::new(fixtures().join("cache"));
    let mut xmls: Vec<String> = SUFFIXES.iter().map(|s| fixture_xml(s)).collect();
    xmls.push(decode(
        &cache,
        &fixtures().join("table/141AC0000000029_20220617_504AC0000000068.json.gz"),
    ));
    xmls
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &to.join(e.file_name()));
        } else {
            std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
        }
    }
}

/// A writable copy of the fixture cache, kept alive by the returned directory.
pub fn fixture_cache() -> (tempfile::TempDir, Cache) {
    let d = tempfile::tempdir().unwrap();
    copy_dir(&fixtures().join("cache"), &d.path().join("cache"));
    let cache = Cache::new(d.path().join("cache"));
    (d, cache)
}
