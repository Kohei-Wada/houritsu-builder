//! Checking a law file of a houritsu checkout against what e-Gov serves now: the revision named in
//! its frontmatter is fetched again and rendered as the build renders it. Nothing is written but a
//! temporary directory for the diff.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context};

use crate::api::Api;
use crate::blob::text;
use crate::fetch::{jst, LAW_DATA_PARAMS};

const DIFF_LINES: usize = 40;

/// The content to check and what the checkout knows about it.
pub struct Local {
    pub path: String,
    pub content: Vec<u8>,
    /// The working-tree file differs from its commit.
    pub edited: bool,
    /// When the checkout got its HEAD commit, as e-Gov writes times; the build ran before it.
    pub received: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum Verdict {
    Match,
    Mismatch { reason: String, diff: String },
}

fn git(dir: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status.success().then_some(out.stdout)
}

fn git_str(dir: &Path, args: &[&str]) -> Option<String> {
    git(dir, args).and_then(|b| String::from_utf8(b).ok())
}

/// A top-level frontmatter field written as a JSON string, e.g. `revision_id: "..."`.
fn field(content: &[u8], key: &str) -> Option<String> {
    let text = std::str::from_utf8(content).ok()?;
    let head = text.strip_prefix("---\n")?.split("\n---\n").next()?;
    let raw = head
        .lines()
        .find_map(|l| l.strip_prefix(&format!("{key}: ")))?;
    serde_json::from_str(raw).ok()
}

fn find_law(repo: &Path, law_id: &str) -> anyhow::Result<PathBuf> {
    let shard: String = law_id.chars().take(3).collect();
    let laws = repo.join("laws");
    for entry in fs::read_dir(&laws).with_context(|| format!("{} has no laws", repo.display()))? {
        let path = entry?.path().join(&shard).join(format!("{law_id}.md"));
        if path.is_file() {
            return Ok(path);
        }
    }
    bail!("no file for {law_id} under {}", laws.display())
}

/// The time the checkout's HEAD became its current commit: the oldest of the newest reflog
/// entries that name it.
fn received(top: &Path) -> Option<String> {
    let head = git_str(top, &["rev-parse", "HEAD"])?;
    let log = git_str(
        top,
        &["log", "-g", "--date=unix", "--format=%H %gd", "HEAD"],
    )?;
    let time = log
        .lines()
        .map_while(|l| l.strip_prefix(head.trim())?.trim().strip_prefix("HEAD@{"))
        .last()?;
    Some(jst(time.trim_end_matches('}').parse().ok()?))
}

/// The file `target` names, a path or a law ID looked up in `repo`, as it is or at `revision`.
pub fn locate(repo: &Path, target: &str, revision: Option<&str>) -> anyhow::Result<Local> {
    let file = if Path::new(target).is_file() {
        PathBuf::from(target)
    } else {
        find_law(repo, target)?
    };
    let dir = fs::canonicalize(&file)?
        .parent()
        .context("a file has a parent")?
        .to_path_buf();
    let top = PathBuf::from(
        git_str(&dir, &["rev-parse", "--show-toplevel"])
            .with_context(|| format!("{} is not in a git checkout", file.display()))?
            .trim(),
    );
    let rel = fs::canonicalize(&file)?
        .strip_prefix(&top)?
        .to_string_lossy()
        .into_owned();
    let received = received(&top);
    let Some(rid) = revision else {
        let status = git_str(&top, &["status", "--porcelain", "--", &rel]).unwrap_or_default();
        return Ok(Local {
            path: rel,
            content: fs::read(&file)?,
            edited: !status.is_empty(),
            received,
        });
    };
    let needle = format!("revision_id: {}", serde_json::to_string(rid)?);
    let commits = git_str(
        &top,
        &["log", "--format=%H", "-S", &needle, "HEAD", "--", &rel],
    )
    .unwrap_or_default();
    for c in commits.lines() {
        if let Some(content) = git(&top, &["show", &format!("{c}:{rel}")]) {
            if field(&content, "revision_id").as_deref() == Some(rid) {
                return Ok(Local {
                    path: format!("{rel} at {}", &c[..12]),
                    content,
                    edited: false,
                    received,
                });
            }
        }
    }
    bail!("{rel} has no revision {rid} in the history of HEAD")
}

fn diff(local: &[u8], egov: &[u8]) -> anyhow::Result<String> {
    let d = tempfile::tempdir()?;
    fs::write(d.path().join("local.md"), local)?;
    fs::write(d.path().join("e-gov.md"), egov)?;
    let out = Command::new("git")
        .args(["diff", "--no-index", "--no-color", "--no-prefix", "-U1"])
        .args(["local.md", "e-gov.md"])
        .current_dir(d.path())
        .output()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = text.lines().collect();
    let mut shown = lines[..lines.len().min(DIFF_LINES)].join("\n");
    if lines.len() > DIFF_LINES {
        shown += &format!("\n... {} more lines", lines.len() - DIFF_LINES);
    }
    Ok(shown)
}

/// Only an edit and an update after the checkout got the file are certain. Otherwise the commit
/// may have been changed, or the build may have used text fetched before e-Gov's update: a daily
/// fetch can miss an update to an old revision until the weekly full one.
fn reason(local: &Local, updated: Option<&str>) -> String {
    if local.edited {
        return "local content differs: the file is not as committed in this checkout".to_string();
    }
    match (updated, &local.received) {
        (Some(u), Some(t)) if u > t.as_str() => format!(
            "e-Gov updated since the build: e-Gov updated this revision at {u}, \
             after this checkout got it at {t}"
        ),
        (Some(u), Some(t)) => format!(
            "the commit differs from e-Gov: e-Gov last updated this revision at {u}, before \
             this checkout got it at {t}; either the commit was changed or the build used text \
             fetched before that update"
        ),
        (Some(u), None) => format!(
            "the commit differs from e-Gov: e-Gov last updated this revision at {u}, and the \
             checkout has no record of when it got it"
        ),
        (None, _) => "the commit differs from e-Gov, which gives no update time".to_string(),
    }
}

/// Renders the revision in `local`'s frontmatter from e-Gov and compares the bytes.
pub fn check(api: &dyn Api, local: &Local) -> anyhow::Result<Verdict> {
    let (law_id, rid) = match (
        field(&local.content, "law_id"),
        field(&local.content, "revision_id"),
    ) {
        (Some(l), Some(r)) => (l, r),
        _ => bail!("{} has no law_id and revision_id frontmatter", local.path),
    };
    let list = api.get_json(&format!("law_revisions/{law_id}"), &[])?;
    let Some(rev) = list["revisions"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|r| r["law_revision_id"] == rid.as_str())
    else {
        return Ok(Verdict::Mismatch {
            reason: format!("e-Gov updated since the build: it no longer lists revision {rid}"),
            diff: String::new(),
        });
    };
    let data = api.get_json(&format!("law_data/{rid}"), LAW_DATA_PARAMS)?;
    let (egov, _) = text(
        &list["law_info"],
        rev,
        data["law_full_text"].as_str().unwrap_or_default(),
        &rid,
    )?;
    if egov.as_bytes() == local.content {
        return Ok(Verdict::Match);
    }
    Ok(Verdict::Mismatch {
        reason: reason(local, rev["updated"].as_str()),
        diff: diff(&local.content, egov.as_bytes())?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ApiError;
    use crate::cache::Cache;
    use crate::testutil::{fixtures, LAW_ID};
    use serde_json::Value;
    use std::collections::HashMap;
    use std::sync::Mutex;

    const RID: &str = "323AC1000000178_20190501_429AC0000000063";
    const OTHER: &str = "323AC1000000178_20200101_430AC1000000057";

    /// e-Gov serving the fixture cache, recording each request.
    struct Fake {
        responses: HashMap<String, Value>,
        calls: Mutex<Vec<String>>,
    }

    impl Fake {
        fn new() -> Self {
            let cache = Cache::new(fixtures().join("cache"));
            let list = cache.read(&cache.revisions_path(LAW_ID)).unwrap();
            let mut responses = HashMap::from([(format!("law_revisions/{LAW_ID}"), list)]);
            for rid in [RID, OTHER] {
                let data = cache.read(&cache.data_path(rid)).unwrap();
                responses.insert(format!("law_data/{rid}"), data);
            }
            Fake {
                responses,
                calls: Mutex::new(vec![]),
            }
        }

        /// e-Gov republishes RID with OTHER's text, as of `updated`.
        fn republish(&mut self, updated: &str) {
            let text = self.responses[&format!("law_data/{OTHER}")]["law_full_text"].clone();
            self.responses.get_mut(&format!("law_data/{RID}")).unwrap()["law_full_text"] = text;
            let list = self
                .responses
                .get_mut(&format!("law_revisions/{LAW_ID}"))
                .unwrap();
            for r in list["revisions"].as_array_mut().unwrap() {
                if r["law_revision_id"] == RID {
                    r["updated"] = Value::from(updated);
                }
            }
        }

        /// RID as the build renders it from what this fake serves.
        fn rendered(&self) -> Vec<u8> {
            let list = &self.responses[&format!("law_revisions/{LAW_ID}")];
            let rev = list["revisions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["law_revision_id"] == RID)
                .unwrap();
            let data = &self.responses[&format!("law_data/{RID}")];
            let full = data["law_full_text"].as_str().unwrap();
            text(&list["law_info"], rev, full, RID)
                .unwrap()
                .0
                .into_bytes()
        }
    }

    impl Api for Fake {
        fn get_json(&self, path: &str, params: &[(&str, &str)]) -> Result<Value, ApiError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{path} {params:?}"));
            self.responses
                .get(path)
                .cloned()
                .ok_or_else(|| ApiError(format!("404 {path}")))
        }
    }

    fn local(content: Vec<u8>) -> Local {
        Local {
            path: "laws/act/323/323AC1000000178.md".to_string(),
            content,
            edited: false,
            received: Some("2026-11-05T04:06:51+09:00".to_string()),
        }
    }

    fn mismatch(v: Verdict) -> (String, String) {
        match v {
            Verdict::Mismatch { reason, diff } => (reason, diff),
            Verdict::Match => panic!("expected a mismatch"),
        }
    }

    #[test]
    fn a_file_as_built_matches_and_fetches_like_the_build() {
        let api = Fake::new();
        assert_eq!(check(&api, &local(api.rendered())).unwrap(), Verdict::Match);
        assert_eq!(
            *api.calls.lock().unwrap(),
            [
                format!("law_revisions/{LAW_ID} []"),
                format!("law_data/{RID} {LAW_DATA_PARAMS:?}"),
            ]
        );
    }

    #[test]
    fn a_changed_file_differs_locally_with_a_diff() {
        let api = Fake::new();
        let content = String::from_utf8(api.rendered()).unwrap();
        let tampered = content.replacen("category: ", "category: \"改ざん\" ", 1);
        let (reason, diff) = mismatch(check(&api, &local(tampered.into_bytes())).unwrap());
        assert!(
            reason.starts_with(
                "the commit differs from e-Gov: e-Gov last updated this revision at \
                 2021-06-18T01:34:13+09:00, before this checkout got it at 2026-11-05T04:06:51"
            ),
            "{reason}"
        );
        assert!(diff.contains("-category: \"改ざん\""), "{diff}");
        assert!(diff.starts_with("diff --git local.md e-gov.md"), "{diff}");
    }

    #[test]
    fn an_edit_in_the_working_tree_is_named_as_such() {
        let api = Fake::new();
        let mut l = local(b"---\nlaw_id: \"323AC1000000178\"\n".to_vec());
        l.content
            .extend(format!("revision_id: \"{RID}\"\n---\n").bytes());
        l.edited = true;
        let (reason, _) = mismatch(check(&api, &l).unwrap());
        assert!(reason.contains("not as committed"), "{reason}");
    }

    #[test]
    fn a_revision_e_gov_updated_after_the_checkout_got_it() {
        let mut api = Fake::new();
        let built = api.rendered();
        api.republish("2026-11-05T15:12:30+09:00");
        let (reason, diff) = mismatch(check(&api, &local(built.clone())).unwrap());
        assert_eq!(
            reason,
            "e-Gov updated since the build: e-Gov updated this revision at \
             2026-11-05T15:12:30+09:00, after this checkout got it at 2026-11-05T04:06:51+09:00"
        );
        assert!(diff.contains("\n@@ "), "{diff}");
        let mut unknown = local(api.rendered());
        unknown.content.push(b'\n');
        unknown.received = None;
        let (reason, _) = mismatch(check(&api, &unknown).unwrap());
        assert!(reason.contains("no record of when"), "{reason}");
        let mut silent = local(built);
        api.responses
            .get_mut(&format!("law_revisions/{LAW_ID}"))
            .unwrap()["revisions"][1]
            .as_object_mut()
            .unwrap()
            .remove("updated");
        silent.received = Some("2000-01-01T00:00:00+09:00".to_string());
        let (reason, _) = mismatch(check(&api, &silent).unwrap());
        assert_eq!(
            reason,
            "the commit differs from e-Gov, which gives no update time"
        );
    }

    #[test]
    fn a_revision_e_gov_no_longer_lists() {
        let api = Fake::new();
        let content = String::from_utf8(api.rendered())
            .unwrap()
            .replace(RID, "323AC1000000178_gone");
        let (reason, diff) = mismatch(check(&api, &local(content.into_bytes())).unwrap());
        assert!(
            reason.contains("no longer lists revision 323AC1000000178_gone"),
            "{reason}"
        );
        assert!(diff.is_empty());
    }

    #[test]
    fn diffs_are_cut_short() {
        let a: String = (0..100).map(|i| format!("a{i}\n")).collect();
        let b: String = (0..100).map(|i| format!("b{i}\n")).collect();
        let d = diff(a.as_bytes(), b.as_bytes()).unwrap();
        assert_eq!(d.lines().count(), DIFF_LINES + 1);
        assert!(d.ends_with("more lines"), "{d}");
    }

    fn commit(repo: &Path, content: &str) {
        let path = repo.join("laws/act/323/323AC1000000178.md");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
        for args in [
            &["add", "-A"][..],
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-qm",
                "c",
            ],
        ] {
            assert!(git(repo, args).is_some(), "git {args:?}");
        }
    }

    #[test]
    fn locates_a_law_by_id_or_path_and_a_revision_in_history() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path();
        git(repo, &["init", "-q", "-b", "main"]).unwrap();
        let old = "---\nlaw_id: \"323AC1000000178\"\nrevision_id: \"R1\"\n---\n\nold\n";
        let new = "---\nlaw_id: \"323AC1000000178\"\nrevision_id: \"R2\"\n---\n\nnew\n";
        commit(repo, old);
        commit(repo, new);

        let by_id = locate(repo, LAW_ID, None).unwrap();
        assert_eq!(by_id.path, "laws/act/323/323AC1000000178.md");
        assert_eq!(by_id.content, new.as_bytes());
        assert!(!by_id.edited);
        assert!(by_id.received.unwrap().ends_with("+09:00"));

        let path = repo.join("laws/act/323/323AC1000000178.md");
        fs::write(&path, "edited").unwrap();
        let by_path = locate(Path::new("/nonexistent"), path.to_str().unwrap(), None).unwrap();
        assert!(by_path.edited);

        let at = locate(repo, LAW_ID, Some("R1")).unwrap();
        assert_eq!(at.content, old.as_bytes());
        assert!(at.path.starts_with("laws/act/323/323AC1000000178.md at "));
        assert!(locate(repo, LAW_ID, Some("R3")).is_err());
        assert!(locate(repo, "999AC0000000001", None).is_err());
    }
}
