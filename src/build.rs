use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::UNIX_EPOCH;

use anyhow::{bail, Context};
use base64::Engine;
use rayon::prelude::*;
use serde_json::{json, Value};
use sha1::{Digest, Sha1};

use crate::cache::Cache;
use crate::fastimport::{hex, write_blob, Commit, Stream};
use crate::fidelity::check_doc;
use crate::frontmatter::frontmatter;
use crate::history::{load_revisions, plan, Group, Revision};
use crate::render::{parse, render_doc, Counts};

const MARKER: &str = ".git/houritsu-build";
const RENDERED: &str = "rendered.json"; // revision → blob of the last build, reused while the inputs are unchanged
pub const RUMDL: [&str; 2] = ["uvx", "rumdl@0.2.79"];
const README: &str = include_str!("../assets/README.md");
const RUMDL_TOML: &str = include_str!("../assets/rumdl.toml");

pub struct Options {
    pub law_type: Option<String>,
    pub lint: bool,
    pub jobs: usize,
    /// The linter command; tests point it at a fake.
    pub rumdl: Vec<String>,
    /// Stands in for the hash of this executable; tests use it to change the code version.
    pub code_version: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            law_type: None,
            lint: true,
            jobs: std::thread::available_parallelism().map_or(1, |n| n.get()),
            rumdl: RUMDL.iter().map(|s| s.to_string()).collect(),
            code_version: None,
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Report {
    pub commits_main: usize,
    pub commits_upcoming: usize,
    pub files: usize,
    pub rendered: usize,
    pub excluded: Vec<String>,
    pub skipped: Vec<String>,
    pub unknown: Counts,
    pub lint: &'static str,
}

impl Report {
    pub fn to_json(&self, with_unknown: bool) -> Value {
        let mut unknown: Vec<(&String, &u64)> = self.unknown.iter().collect();
        unknown.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let mut v = json!({
            "commits_main": self.commits_main,
            "commits_upcoming": self.commits_upcoming,
            "files": self.files,
            "rendered": self.rendered,
            "excluded": self.excluded,
            "skipped": self.skipped,
        });
        if with_unknown {
            v["unknown"] = Value::Object(unknown.into_iter().map(|(k, n)| (k.clone(), json!(n))).collect());
        }
        v["lint"] = json!(self.lint);
        v
    }
}

fn git(dir: &Path, args: &[&str]) -> anyhow::Result<String> {
    let out = Command::new("git").args(args).current_dir(dir).stderr(Stdio::inherit()).output()?;
    if !out.status.success() {
        bail!("git {} failed in {}", args.join(" "), dir.display());
    }
    Ok(String::from_utf8(out.stdout)?)
}

fn clear(path: &Path) -> anyhow::Result<()> {
    if path.exists() && fs::read_dir(path)?.next().is_some() {
        if !path.join(MARKER).exists() {
            bail!("{} is not empty and not a previous build; refusing to touch it", path.display());
        }
        fs::remove_dir_all(path)?;
    }
    fs::create_dir_all(path)?;
    Ok(())
}

fn prepare(staging: &Path) -> anyhow::Result<()> {
    clear(staging)?;
    git(staging, &["init", "-q", "-b", "main", "."])?;
    fs::write(staging.join(MARKER), "created by houritsu-builder\n")?;
    Ok(())
}

/// Everything a blob or its lint result depends on. Fetch or CLI changes keep the previous renders.
const RENDER_SOURCES: [&str; 7] = [
    include_str!("render.rs"),
    include_str!("fidelity.rs"),
    include_str!("frontmatter.rs"),
    include_str!("xmltext.rs"),
    include_str!("build.rs"),
    include_str!("../assets/rumdl.toml"),
    include_str!("../Cargo.lock"),
];

fn code_version() -> &'static str {
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION.get_or_init(|| {
        let mut h = Sha1::new();
        h.update(RUMDL.join(" "));
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

fn input_key(cache: &Cache, r: &Revision) -> anyhow::Result<String> {
    let st = fs::metadata(cache.data_path(r.revision_id()))?;
    let mtime = st.modified()?.duration_since(UNIX_EPOCH)?.as_nanos();
    let meta = canonical(&json!([&*r.law_info, &r.rev]));
    Ok(hex(&Sha1::digest(format!("{} {mtime} {meta}", st.len()))))
}

type Entry = (String, String, Counts);

fn load_rendered(cache: &Cache, out: &Path, version: &str) -> anyhow::Result<HashMap<String, Entry>> {
    let path = cache.root.join(RENDERED);
    if !path.exists() || !out.join(MARKER).exists() {
        return Ok(HashMap::new());
    }
    let doc: Value = serde_json::from_str(&fs::read_to_string(&path)?)?;
    if doc["version"] != version {
        return Ok(HashMap::new());
    }
    let entries: HashMap<String, Entry> = serde_json::from_value(doc["entries"].clone())?;
    let mut p = Command::new("git")
        .args(["cat-file", "--batch-check=%(objectname)"])
        .current_dir(out)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let input: String = entries.values().map(|e| format!("{}\n", e.1)).collect();
    let mut stdin = p.stdin.take().unwrap();
    let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let output = p.wait_with_output()?;
    writer.join().unwrap()?;
    let present: HashSet<&str> = std::str::from_utf8(&output.stdout)?.lines().filter(|l| !l.ends_with(" missing")).collect();
    Ok(entries.into_iter().filter(|(_, e)| present.contains(e.1.as_str())).collect())
}

fn render(cache: &Cache, objects: &Path, r: &Revision, lint_file: Option<&Path>) -> anyhow::Result<(String, Counts)> {
    #[derive(serde::Deserialize)]
    struct Data<'a> {
        #[serde(borrow)]
        law_full_text: Option<std::borrow::Cow<'a, str>>,
    }
    let raw = cache.read_raw(&cache.data_path(r.revision_id()))?;
    let data: Data = serde_json::from_slice(&raw).with_context(|| r.revision_id().to_string())?;
    let b64 = data.law_full_text.as_deref().unwrap_or_default();
    let engine = &base64::engine::general_purpose::STANDARD;
    let bytes = engine.decode(b64).or_else(|_| engine.decode(b64.split_ascii_whitespace().collect::<String>()))?;
    let xml = std::str::from_utf8(&bytes)?;
    let xml = xml.strip_prefix('\u{feff}').unwrap_or(xml);
    let doc = parse(xml).with_context(|| r.revision_id().to_string())?;
    let rendered = render_doc(&doc).with_context(|| r.revision_id().to_string())?;
    check_doc(&doc, &rendered.markdown, r.revision_id())?;
    let text = frontmatter(&r.law_info, &r.rev) + &rendered.markdown;
    if let Some(f) = lint_file {
        fs::create_dir_all(f.parent().unwrap())?;
        fs::write(f, &text)?;
    }
    Ok((write_blob(objects, text.as_bytes())?, rendered.unknown))
}

fn render_all(cache: &Cache, objects: &Path, todo: &[(&Revision, Option<PathBuf>)], jobs: usize) -> anyhow::Result<Vec<(String, Counts)>> {
    let pool = rayon::ThreadPoolBuilder::new().num_threads(jobs.max(1)).build()?;
    pool.install(|| todo.par_iter().map(|(r, lint)| render(cache, objects, r, lint.as_deref())).collect())
}

struct Streamed {
    files: usize,
    entries: BTreeMap<String, Entry>,
    unknown: Counts,
}

fn stream(cache: &Cache, staging: &Path, main: &[Group], upcoming: &[Group], lint_dir: Option<&Path>, jobs: usize, previous: &HashMap<String, Entry>) -> anyhow::Result<Streamed> {
    let all: Vec<&Revision> = main.iter().chain(upcoming).flat_map(|g| &g.revisions).filter(|r| !r.repealed()).collect();
    let mut keys = HashMap::new();
    for r in &all {
        keys.insert(r.revision_id().to_string(), input_key(cache, r)?);
    }
    let hits: HashMap<&str, &Entry> =
        all.iter().filter_map(|r| previous.get(r.revision_id()).filter(|e| e.0 == keys[r.revision_id()]).map(|e| (r.revision_id(), e))).collect();
    let todo: Vec<(&Revision, Option<PathBuf>)> = all
        .iter()
        .filter(|r| !hits.contains_key(r.revision_id()))
        .map(|r| (*r, lint_dir.map(|d| d.join(r.path().trim_end_matches(".md")).join(format!("{}.md", r.revision_id())))))
        .collect();
    let mut rendered = render_all(cache, &staging.join(".git/objects"), &todo, jobs)?.into_iter();

    let mut proc = Command::new("git").args(["fast-import", "--quiet", "--done"]).current_dir(staging).stdin(Stdio::piped()).spawn()?;
    let result = (|| -> anyhow::Result<Streamed> {
        let mut s = Stream::new(BufWriter::new(proc.stdin.take().unwrap()));
        let root_date = main.first().map_or("1970-01-02".to_string(), Group::commit_date);
        s.commit(Commit {
            ref_: "refs/heads/main",
            date: &root_date,
            message: "リポジトリの初期化\n",
            files: BTreeMap::from([("README.md".to_string(), README.to_string()), (".rumdl.toml".to_string(), RUMDL_TOML.to_string())]),
            ..Default::default()
        })?;
        let mut out = Streamed { files: 0, entries: BTreeMap::new(), unknown: Counts::new() };
        for (ref_, groups) in [("refs/heads/main", main), ("refs/heads/upcoming", upcoming)] {
            for (i, g) in groups.iter().enumerate() {
                let (mut blobs, mut deleted) = (BTreeMap::new(), BTreeSet::new());
                for r in &g.revisions {
                    if r.repealed() {
                        deleted.insert(r.path());
                        continue;
                    }
                    let (sha, found) = match hits.get(r.revision_id()) {
                        Some(e) => (e.1.clone(), e.2.clone()),
                        None => rendered.next().expect("one render per revision"),
                    };
                    for (k, n) in &found {
                        *out.unknown.entry(k.clone()).or_default() += n;
                    }
                    blobs.insert(r.path(), sha.clone());
                    out.entries.insert(r.revision_id().to_string(), (keys[r.revision_id()].clone(), sha, found));
                    out.files += 1;
                }
                let (date, message, author) = (g.commit_date(), g.message(), g.author());
                s.commit(Commit {
                    ref_,
                    date: &date,
                    message: &message,
                    blobs,
                    deleted,
                    from_ref: (ref_.ends_with("upcoming") && i == 0).then_some("refs/heads/main"),
                    author: Some(&author),
                    ..Default::default()
                })?;
            }
        }
        s.done()?;
        Ok(out)
    })();
    if result.is_err() {
        let _ = proc.kill();
    }
    let status = proc.wait()?;
    let out = result?;
    if !status.success() {
        bail!("git fast-import failed");
    }
    Ok(out)
}

fn copy_remote_config(old: &Path, new: &Path) -> anyhow::Result<()> {
    if !old.join(".git").exists() {
        return Ok(());
    }
    let found = Command::new("git").args(["config", "--local", "--get-regexp", r"^(remote|branch)\."]).current_dir(old).output()?;
    for line in String::from_utf8(found.stdout)?.lines() {
        if let Some((key, value)) = line.split_once(' ') {
            git(new, &["config", "--add", key, value])?;
        }
    }
    Ok(())
}

fn lint(lint_dir: &Path, rumdl: &[String]) -> anyhow::Result<(bool, String)> {
    fs::create_dir_all(lint_dir)?;
    fs::write(lint_dir.join(".rumdl.toml"), RUMDL_TOML)?;
    let out = Command::new(&rumdl[0])
        .args(&rumdl[1..])
        .args(["check", "--no-cache", "--respect-gitignore=false", "."])
        .current_dir(lint_dir)
        .output()
        .with_context(|| format!("running {}", rumdl.join(" ")))?;
    Ok((out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr)))
}

pub fn build(cache: &Cache, out: &Path, o: &Options) -> anyhow::Result<Report> {
    let name = out.file_name().context("output path has no name")?.to_string_lossy();
    let staging = out.with_file_name(format!("{name}.building"));
    if out.exists() && fs::read_dir(out)?.next().is_some() && !out.join(MARKER).exists() {
        bail!("{} is not empty and not a previous build; refusing to touch it", out.display());
    }
    prepare(&staging)?;
    let version = o.code_version.clone().unwrap_or_else(|| code_version().to_string());
    let lint_dir = cache.root.join("lint");
    let result = (|| -> anyhow::Result<Report> {
        let previous = load_rendered(cache, out, &version)?;
        let alternates = staging.join(".git/objects/info/alternates");
        if !previous.is_empty() {
            fs::write(&alternates, format!("{}\n", fs::canonicalize(out.join(".git/objects"))?.display()))?;
        }
        let loaded = load_revisions(cache, o.law_type.as_deref())?;
        let (main, upcoming) = plan(loaded.revisions);
        if o.lint {
            let _ = fs::remove_dir_all(&lint_dir);
            fs::create_dir_all(&lint_dir)?;
        }
        let streamed = stream(cache, &staging, &main, &upcoming, o.lint.then_some(lint_dir.as_path()), o.jobs, &previous)?;
        git(&staging, &["reset", "-q", "--hard", "main"])?;
        // Loose and borrowed blobs → one delta-compressed pack, while lint runs on the same files.
        let mut repack = Command::new("git").args(["repack", "-adq"]).current_dir(&staging).spawn()?;
        let linted = if o.lint { Some(lint(&lint_dir, &o.rumdl)) } else { None };
        if !repack.wait()?.success() {
            bail!("git repack failed");
        }
        fs::remove_file(&alternates).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) })?;
        let mut report = Report {
            commits_main: main.len(),
            commits_upcoming: upcoming.len(),
            files: streamed.files,
            rendered: streamed.entries.iter().filter(|(rid, e)| previous.get(*rid) != Some(e)).count(),
            excluded: loaded.excluded,
            skipped: loaded.skipped,
            unknown: streamed.unknown,
            lint: "skipped",
        };
        if let Some(linted) = linted {
            let (ok, output) = linted?;
            if !ok {
                fs::write(cache.root.join("lint.log"), &output)?;
                let head: String = output.chars().take(4000).collect();
                bail!("rumdl failed (full log: lint.log in cache):\n{head}");
            }
            report.lint = "ok";
        }
        copy_remote_config(out, &staging)?;
        clear(out)?;
        fs::remove_dir(out)?;
        fs::rename(&staging, out)?;
        if o.lint {
            // entries from a --no-lint build were never linted, so they must not skip lint later
            let doc = json!({"version": version, "entries": streamed.entries});
            fs::write(cache.root.join(RENDERED), serde_json::to_string(&doc)?)?;
        }
        Ok(report)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    let _ = fs::remove_dir_all(&lint_dir);
    let report = result?;
    fs::create_dir_all(&cache.root)?;
    fs::write(cache.root.join("build-report.json"), serde_json::to_string_pretty(&report.to_json(true))?)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::render_law;
    use crate::testutil::{fixture_cache, LAW_ID};

    fn sh(repo: &Path, args: &[&str]) -> String {
        git(repo, args).unwrap()
    }

    fn opts(lint: bool, jobs: usize) -> Options {
        Options { lint, jobs, ..Default::default() }
    }

    #[test]
    fn builds_history_from_fixture() {
        let (d, cache) = fixture_cache();
        let out = d.path().join("out");
        let report = build(&cache, &out, &opts(false, 2)).unwrap();
        let subjects = sh(&out, &["log", "--topo-order", "--format=%ad %s", "--date=short", "main"]);
        let subjects: Vec<&str> = subjects.lines().collect();
        assert_eq!(subjects.len(), 4); // root + 3 amendments
        assert!(subjects[3].ends_with("リポジトリの初期化"));
        assert_eq!(subjects[..3].iter().map(|s| s.split(' ').next().unwrap()).collect::<Vec<_>>(), ["2020-01-01", "2019-05-01", "2016-01-01"]);
        let md = sh(&out, &["show", "main:laws/act/323/323AC1000000178.md"]);
        assert!(md.starts_with("---\nlaw_id: \"323AC1000000178\"\n") && md.contains("# 国民の祝日に関する法律"));
        assert!(report.commits_main == 3 && report.excluded.is_empty() && report.skipped.is_empty());
        assert!(git(&out, &["rev-parse", "--verify", "-q", "upcoming"]).is_err());
        assert!(out.join("laws/act/323/323AC1000000178.md").exists()); // working tree checked out
    }

    #[test]
    fn build_is_deterministic() {
        let (d, cache) = fixture_cache();
        build(&cache, &d.path().join("a"), &opts(false, 1)).unwrap();
        build(&cache, &d.path().join("b"), &opts(false, 4)).unwrap();
        assert_eq!(sh(&d.path().join("a"), &["rev-parse", "main"]), sh(&d.path().join("b"), &["rev-parse", "main"]));
    }

    #[test]
    fn refuses_unknown_nonempty_dir() {
        let (d, cache) = fixture_cache();
        let out = d.path().join("out");
        fs::create_dir(&out).unwrap();
        fs::write(out.join("precious.txt"), "keep").unwrap();
        assert!(build(&cache, &out, &opts(false, 1)).unwrap_err().to_string().contains("not a previous build"));
        assert_eq!(fs::read_to_string(out.join("precious.txt")).unwrap(), "keep");
    }

    #[test]
    fn rebuild_replaces_previous_build_and_keeps_remote_config() {
        let (d, cache) = fixture_cache();
        let out = d.path().join("out");
        build(&cache, &out, &opts(false, 1)).unwrap();
        sh(&out, &["remote", "add", "origin", "git@example.com:x/houritsu.git"]);
        sh(&out, &["config", "branch.main.remote", "origin"]);
        build(&cache, &out, &opts(false, 1)).unwrap();
        assert_eq!(sh(&out, &["log", "--format=%s", "main"]).lines().count(), 4);
        assert_eq!(sh(&out, &["remote", "get-url", "origin"]), "git@example.com:x/houritsu.git\n");
        assert_eq!(sh(&out, &["config", "branch.main.remote"]), "origin\n");
    }

    #[test]
    fn rumdl_passes_on_every_revision() {
        let (d, cache) = fixture_cache();
        assert_eq!(build(&cache, &d.path().join("out"), &opts(true, 2)).unwrap().lint, "ok");
    }

    #[test]
    fn lint_failure_keeps_previous_build_and_leaves_no_staging() {
        let (d, cache) = fixture_cache();
        let out = d.path().join("out");
        build(&cache, &out, &opts(false, 1)).unwrap();
        let before = sh(&out, &["rev-parse", "main"]);
        let fake = ["sh", "-c", "head -c 5000 /dev/zero | tr '\\0' x; exit 1", "rumdl"];
        let o = Options { rumdl: fake.iter().map(|s| s.to_string()).collect(), ..opts(true, 1) };
        assert!(build(&cache, &out, &o).unwrap_err().to_string().contains("rumdl failed"));
        assert_eq!(sh(&out, &["rev-parse", "main"]), before);
        assert!(!d.path().join("out.building").exists() && !cache.root.join("lint").exists());
        assert_eq!(fs::read_to_string(cache.root.join("lint.log")).unwrap().len(), 5000);
    }

    fn edit_revisions(cache: &Cache, f: impl FnOnce(&mut Vec<Value>)) {
        let path = cache.revisions_path(LAW_ID);
        let mut doc = cache.read(&path).unwrap();
        f(doc["revisions"].as_array_mut().unwrap());
        cache.write(&path, &doc).unwrap();
    }

    #[test]
    fn upcoming_branch_descends_from_main() {
        let (d, cache) = fixture_cache();
        let mut last = Value::Null;
        edit_revisions(&cache, |revs| {
            last = revs.last().unwrap().clone();
            let mut new = last.clone();
            new["law_revision_id"] = json!(format!("{LAW_ID}_20300101_999AC0000000001"));
            new["amendment_enforcement_date"] = json!("2030-01-01");
            new["amendment_law_id"] = json!("999AC0000000001");
            new["current_revision_status"] = json!("UnEnforced");
            revs.push(new);
        });
        let data = cache.read(&cache.data_path(last["law_revision_id"].as_str().unwrap())).unwrap();
        cache.write(&cache.data_path(&format!("{LAW_ID}_20300101_999AC0000000001")), &data).unwrap();
        let out = d.path().join("out");
        assert_eq!(build(&cache, &out, &opts(false, 1)).unwrap().commits_upcoming, 1);
        sh(&out, &["merge-base", "--is-ancestor", "main", "upcoming"]);
        assert!(!sh(&out, &["log", "--format=%B", "main"]).contains("999AC0000000001"));
        assert!(sh(&out, &["log", "--format=%B", "-1", "upcoming"]).contains("999AC0000000001"));
    }

    #[test]
    fn repeal_removes_the_file() {
        let (d, cache) = fixture_cache();
        edit_revisions(&cache, |revs| {
            let mut r = revs[0].clone();
            r["law_revision_id"] = json!(format!("{LAW_ID}_20250401_999AC0000000001"));
            r["amendment_enforcement_date"] = json!("2025-04-01");
            r["amendment_law_id"] = json!("999AC0000000001");
            r["amendment_law_title"] = json!("国民の祝日に関する法律を廃止する法律");
            r["current_revision_status"] = json!("Repeal");
            r["repeal_status"] = json!("Repeal");
            revs.insert(0, r);
        });
        let out = d.path().join("out");
        assert_eq!(build(&cache, &out, &opts(false, 1)).unwrap().commits_main, 4);
        assert_eq!(sh(&out, &["log", "-1", "--format=%s", "main"]), "国民の祝日に関する法律を廃止する法律 施行\n");
        assert_eq!(sh(&out, &["ls-tree", "-r", "--name-only", "main", "laws"]), "");
        assert!(!out.join(format!("laws/act/323/{LAW_ID}.md")).exists());
        assert_eq!(sh(&out, &["log", "--format=%h", "main", "--", &format!("laws/act/323/{LAW_ID}.md")]).split_whitespace().count(), 4);
    }

    #[test]
    fn worker_error_propagates_and_cleans_up() {
        let (d, cache) = fixture_cache();
        let rid = cache.read(&cache.revisions_path(LAW_ID)).unwrap()["revisions"][0]["law_revision_id"].as_str().unwrap().to_string();
        let mut doc = cache.read(&cache.data_path(&rid)).unwrap();
        doc["law_full_text"] = json!(base64::engine::general_purpose::STANDARD.encode("<Law>"));
        cache.write(&cache.data_path(&rid), &doc).unwrap();
        assert!(build(&cache, &d.path().join("out"), &opts(false, 2)).is_err());
        assert!(!d.path().join("out.building").exists() && !d.path().join("out").exists());
    }

    #[test]
    fn rebuild_reuses_blobs_of_the_previous_build() {
        let (d, cache) = fixture_cache();
        let out = d.path().join("out");
        let first = build(&cache, &out, &opts(true, 1)).unwrap();
        let head = sh(&out, &["rev-parse", "main"]);
        let second = build(&cache, &out, &opts(true, 1)).unwrap();
        assert!(first.rendered == first.files && first.files > 0 && second.rendered == 0);
        assert_eq!(sh(&out, &["rev-parse", "main"]), head);
        assert!(!out.join(".git/objects/info/alternates").exists());
        sh(&out, &["fsck", "--strict", "--no-dangling"]);
        let changed = Options { code_version: Some("changed".into()), ..opts(true, 1) };
        assert_eq!(build(&cache, &out, &changed).unwrap().rendered, first.files);
    }

    #[test]
    fn rebuild_renders_again_when_the_previous_build_is_gone() {
        let (d, cache) = fixture_cache();
        build(&cache, &d.path().join("a"), &opts(true, 1)).unwrap();
        assert!(build(&cache, &d.path().join("b"), &opts(true, 1)).unwrap().rendered > 0);
    }

    #[test]
    fn no_lint_build_does_not_let_later_builds_skip_lint() {
        let (d, cache) = fixture_cache();
        let out = d.path().join("out");
        let first = build(&cache, &out, &opts(false, 1)).unwrap();
        assert_eq!(build(&cache, &out, &opts(true, 1)).unwrap().rendered, first.files);
    }

    #[test]
    fn rebuild_renders_a_revision_whose_data_changed() {
        let (d, cache) = fixture_cache();
        let out = d.path().join("out");
        build(&cache, &out, &opts(true, 1)).unwrap();
        let rid = cache.read(&cache.revisions_path(LAW_ID)).unwrap()["revisions"][0]["law_revision_id"].as_str().unwrap().to_string();
        let path = cache.data_path(&rid);
        let f = fs::File::options().write(true).open(&path).unwrap();
        f.set_modified(fs::metadata(&path).unwrap().modified().unwrap() + std::time::Duration::from_nanos(1)).unwrap();
        assert_eq!(build(&cache, &out, &opts(true, 1)).unwrap().rendered, 1);
    }

    #[test]
    fn table_fixture_passes_shipped_rumdl_config() {
        let d = tempfile::tempdir().unwrap();
        let xml = crate::testutil::fixture_xmls().pop().unwrap();
        let target = d.path().join("laws/act/x.md");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, render_law(&xml).unwrap().markdown).unwrap();
        let rumdl: Vec<String> = RUMDL.iter().map(|s| s.to_string()).collect();
        let (ok, output) = lint(d.path(), &rumdl).unwrap();
        assert!(ok, "{output}");
    }

    #[test]
    fn shipped_rumdl_config_rejects_a_skipped_heading_level() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("laws/act/x.md");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, "# 法\n\n### 第一条\n\n本文\n").unwrap();
        let rumdl: Vec<String> = RUMDL.iter().map(|s| s.to_string()).collect();
        let (ok, output) = lint(d.path(), &rumdl).unwrap();
        assert!(!ok && output.contains("MD001"), "{output}");
    }
}
