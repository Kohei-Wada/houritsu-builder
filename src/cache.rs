use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::Context;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde_json::{json, Value};

#[derive(Clone)]
pub struct Cache {
    pub root: PathBuf,
}

impl Cache {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Cache { root: root.into() }
    }

    pub fn laws_path(&self) -> PathBuf {
        self.root.join("laws.json.gz")
    }

    pub fn revisions_path(&self, law_id: &str) -> PathBuf {
        self.root
            .join("revisions")
            .join(format!("{law_id}.json.gz"))
    }

    pub fn data_path(&self, revision_id: &str) -> PathBuf {
        let law_id = revision_id.split('_').next().unwrap_or(revision_id);
        self.root
            .join("data")
            .join(law_id)
            .join(format!("{revision_id}.json.gz"))
    }

    pub fn write(&self, path: &Path, value: &Value) -> anyhow::Result<()> {
        fs::create_dir_all(path.parent().unwrap())?;
        let tmp = path.with_file_name(format!(
            "{}.tmp",
            path.file_name().unwrap().to_string_lossy()
        ));
        let mut gz = GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&serde_json::to_vec(value)?)?;
        fs::write(&tmp, gz.finish()?)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn read(&self, path: &Path) -> anyhow::Result<Value> {
        serde_json::from_slice(&self.read_raw(path)?).with_context(|| path.display().to_string())
    }

    /// The decompressed JSON, for callers that deserialise it without building a `Value`.
    pub fn read_raw(&self, path: &Path) -> anyhow::Result<Vec<u8>> {
        let gz = fs::read(path).with_context(|| path.display().to_string())?;
        // The gzip trailer holds the size mod 2^32; reserving it avoids regrowing a 40 MB buffer.
        let size = gz.len().checked_sub(4).map_or(0, |i| {
            u32::from_le_bytes(gz[i..].try_into().unwrap()) as usize
        });
        // a damaged file's trailer is garbage: never reserve more than real JSON would need
        let mut raw = Vec::with_capacity(size.min(gz.len().saturating_mul(64)));
        GzDecoder::new(gz.as_slice())
            .read_to_end(&mut raw)
            .with_context(|| path.display().to_string())?;
        Ok(raw)
    }

    pub fn record_failure(&self, kind: &str, key: &str, error: &str) -> anyhow::Result<()> {
        fs::create_dir_all(&self.root)?;
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.root.join("failures.jsonl"))?;
        writeln!(f, "{}", json!({"kind": kind, "key": key, "error": error}))?;
        Ok(())
    }

    pub fn revision_files(&self) -> anyhow::Result<Vec<PathBuf>> {
        let dir = self.root.join("revisions");
        if !dir.exists() {
            return Ok(vec![]);
        }
        let mut files: Vec<PathBuf> = fs::read_dir(dir)?
            .map(|e| e.map(|e| e.path()))
            .collect::<Result<_, _>>()?;
        files.retain(|p| p.to_string_lossy().ends_with(".json.gz"));
        files.sort();
        Ok(files)
    }

    /// Revisions whose full text e-Gov answered with 404: they will never come.
    pub fn not_found(&self) -> anyhow::Result<HashSet<String>> {
        let path = self.root.join("failures.jsonl");
        if !path.exists() {
            return Ok(HashSet::new());
        }
        let mut found = HashSet::new();
        for line in fs::read_to_string(path)?
            .lines()
            .filter(|l| !l.trim().is_empty())
        {
            // a line cut short by a killed run says nothing
            let Ok(f) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let error = match &f["error"] {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            if f["kind"] == "law_data" && error.starts_with("404") {
                found.insert(f["key"].as_str().unwrap_or_default().to_string());
            }
        }
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout() {
        let c = Cache::new("/c");
        assert_eq!(c.laws_path(), PathBuf::from("/c/laws.json.gz"));
        assert_eq!(
            c.revisions_path("415AC0000000057"),
            PathBuf::from("/c/revisions/415AC0000000057.json.gz")
        );
        let rid = "415AC0000000057_20261001_507AC0000000070";
        assert_eq!(
            c.data_path(rid),
            PathBuf::from(format!("/c/data/415AC0000000057/{rid}.json.gz"))
        );
    }

    #[test]
    fn write_read_roundtrip_and_no_temp_left() {
        let d = tempfile::tempdir().unwrap();
        let c = Cache::new(d.path());
        let p = c.revisions_path("X");
        c.write(&p, &json!({"a": "全角　空白"})).unwrap();
        assert_eq!(c.read(&p).unwrap(), json!({"a": "全角　空白"}));
        let names: Vec<_> = fs::read_dir(p.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["X.json.gz"]);
    }

    #[test]
    fn record_failure_appends() {
        let d = tempfile::tempdir().unwrap();
        let c = Cache::new(d.path());
        c.record_failure("law_data", "R1", "boom").unwrap();
        c.record_failure("law_revisions", "L1", "boom2").unwrap();
        let keys: Vec<String> = fs::read_to_string(d.path().join("failures.jsonl"))
            .unwrap()
            .lines()
            .map(|l| {
                serde_json::from_str::<Value>(l).unwrap()["key"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(keys, ["R1", "L1"]);
    }

    #[test]
    fn revision_files_sorted() {
        let d = tempfile::tempdir().unwrap();
        let c = Cache::new(d.path());
        for law in ["B", "A"] {
            c.write(&c.revisions_path(law), &json!({})).unwrap();
        }
        let names: Vec<_> = c
            .revision_files()
            .unwrap()
            .iter()
            .map(|p| p.file_name().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["A.json.gz", "B.json.gz"]);
    }

    #[test]
    fn not_found_reads_only_404_law_data_failures() {
        let d = tempfile::tempdir().unwrap();
        let c = Cache::new(d.path());
        assert!(c.not_found().unwrap().is_empty());
        c.record_failure("law_data", "A_1_X", "404 u").unwrap();
        c.record_failure("law_data", "A_2_X", "gave up after 5 attempts")
            .unwrap();
        c.record_failure("revisions", "B", "404 u").unwrap();
        assert_eq!(c.not_found().unwrap(), HashSet::from(["A_1_X".to_string()]));
    }
}
