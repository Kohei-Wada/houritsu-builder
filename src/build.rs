use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context};
use base64::Engine;
use rayon::prelude::*;
use serde_json::{json, Value};

use crate::bills::Bills;
use crate::cache::Cache;
use crate::fastimport::{write_blob, Commit, Stream};
use crate::fidelity::check_doc;
use crate::frontmatter::frontmatter;
use crate::history::{load_revisions, plan, Group, Revision};
use crate::render::{parse, render_doc, Counts};
use crate::reuse::{self, Entries, Entry};
use crate::{git, lint};

const MARKER: &str = ".git/houritsu-build";
const README: &str = include_str!("../assets/README.md");

pub struct Options {
    pub law_type: Option<String>,
    pub lint: bool,
    pub jobs: usize,
    /// The linter command; tests point it at a fake.
    pub rumdl: Vec<String>,
    /// Stands in for the hash of the rendering sources; tests use it to change the code version.
    pub code_version: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            law_type: None,
            lint: true,
            jobs: std::thread::available_parallelism().map_or(1, |n| n.get()),
            rumdl: lint::RUMDL.iter().map(|s| s.to_string()).collect(),
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
            v["unknown"] = Value::Object(
                unknown
                    .into_iter()
                    .map(|(k, n)| (k.clone(), json!(n)))
                    .collect(),
            );
        }
        v["lint"] = json!(self.lint);
        v
    }
}

fn clear(path: &Path) -> anyhow::Result<()> {
    if path.exists() && fs::read_dir(path)?.next().is_some() {
        if !path.join(MARKER).exists() {
            bail!(
                "{} is not empty and not a previous build; refusing to touch it",
                path.display()
            );
        }
        fs::remove_dir_all(path)?;
    }
    fs::create_dir_all(path)?;
    Ok(())
}

fn prepare(staging: &Path) -> anyhow::Result<()> {
    clear(staging)?;
    git::run(staging, &["init", "-q", "-b", "main", "."])?;
    fs::write(staging.join(MARKER), "created by houritsu-builder\n")?;
    Ok(())
}

fn render(
    cache: &Cache,
    objects: &Path,
    r: &Revision,
    lint_file: Option<&Path>,
) -> anyhow::Result<(String, Counts)> {
    #[derive(serde::Deserialize)]
    struct Data<'a> {
        #[serde(borrow)]
        law_full_text: Option<std::borrow::Cow<'a, str>>,
    }
    let raw = cache.read_raw(&cache.data_path(r.revision_id()))?;
    let data: Data = serde_json::from_slice(&raw).with_context(|| r.revision_id().to_string())?;
    let b64 = data.law_full_text.as_deref().unwrap_or_default();
    let engine = &base64::engine::general_purpose::STANDARD;
    let bytes = engine
        .decode(b64)
        .or_else(|_| engine.decode(b64.split_ascii_whitespace().collect::<String>()))?;
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

fn render_all(
    cache: &Cache,
    objects: &Path,
    todo: &[(&Revision, Option<PathBuf>)],
    jobs: usize,
) -> anyhow::Result<Vec<(String, Counts)>> {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs.max(1))
        .build()?;
    pool.install(|| {
        todo.par_iter()
            .map(|(r, lint)| render(cache, objects, r, lint.as_deref()))
            .collect()
    })
}

struct Streamed {
    files: usize,
    entries: Entries,
    unknown: Counts,
}

/// The commits to write: groups of revisions and the bills behind amending Acts.
#[derive(Clone, Copy)]
struct History<'a> {
    main: &'a [Group],
    upcoming: &'a [Group],
    bills: &'a Bills,
}

fn stream(
    cache: &Cache,
    staging: &Path,
    history: &History,
    lint_dir: Option<&Path>,
    jobs: usize,
    previous: &HashMap<String, Entry>,
) -> anyhow::Result<Streamed> {
    let History { main, upcoming, bills } = *history;
    let all: Vec<&Revision> = main
        .iter()
        .chain(upcoming)
        .flat_map(|g| &g.revisions)
        .filter(|r| !r.repealed())
        .collect();
    let mut keys = HashMap::new();
    for r in &all {
        keys.insert(r.revision_id().to_string(), reuse::input_key(cache, r)?);
    }
    let hits: HashMap<&str, &Entry> = all
        .iter()
        .filter_map(|r| {
            previous
                .get(r.revision_id())
                .filter(|e| e.0 == keys[r.revision_id()])
                .map(|e| (r.revision_id(), e))
        })
        .collect();
    let todo: Vec<(&Revision, Option<PathBuf>)> = all
        .iter()
        .filter(|r| !hits.contains_key(r.revision_id()))
        .map(|r| {
            (
                *r,
                lint_dir.map(|d| {
                    d.join(r.path().trim_end_matches(".md"))
                        .join(format!("{}.md", r.revision_id()))
                }),
            )
        })
        .collect();
    let mut rendered = render_all(cache, &staging.join(".git/objects"), &todo, jobs)?.into_iter();

    let mut proc = Command::new("git")
        .args(["fast-import", "--quiet", "--done"])
        .current_dir(staging)
        .stdin(Stdio::piped())
        .spawn()?;
    let result = (|| -> anyhow::Result<Streamed> {
        let mut s = Stream::new(BufWriter::new(proc.stdin.take().unwrap()));
        let root_date = main
            .first()
            .map_or("1970-01-02".to_string(), Group::commit_date);
        s.commit(Commit {
            ref_: "refs/heads/main",
            date: &root_date,
            message: "リポジトリの初期化\n",
            files: BTreeMap::from([
                ("README.md".to_string(), README.to_string()),
                (".rumdl.toml".to_string(), lint::CONFIG.to_string()),
            ]),
            ..Default::default()
        })?;
        let mut out = Streamed {
            files: 0,
            entries: BTreeMap::new(),
            unknown: Counts::new(),
        };
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
                    out.entries.insert(
                        r.revision_id().to_string(),
                        (keys[r.revision_id()].clone(), sha, found),
                    );
                    out.files += 1;
                }
                let (date, message, author) = (g.commit_date(), g.message(bills), g.author());
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

pub fn build(cache: &Cache, out: &Path, o: &Options) -> anyhow::Result<Report> {
    let name = out
        .file_name()
        .context("output path has no name")?
        .to_string_lossy();
    let staging = out.with_file_name(format!("{name}.building"));
    if out.exists() && fs::read_dir(out)?.next().is_some() && !out.join(MARKER).exists() {
        bail!(
            "{} is not empty and not a previous build; refusing to touch it",
            out.display()
        );
    }
    prepare(&staging)?;
    let version = o
        .code_version
        .clone()
        .unwrap_or_else(|| reuse::code_version().to_string());
    let lint_dir = cache.root.join("lint");
    let result = (|| -> anyhow::Result<Report> {
        let previous = if out.join(MARKER).exists() {
            reuse::load(cache, out, &version)?
        } else {
            HashMap::new()
        };
        let alternates = staging.join(".git/objects/info/alternates");
        if !previous.is_empty() {
            fs::write(
                &alternates,
                format!(
                    "{}\n",
                    fs::canonicalize(out.join(".git/objects"))?.display()
                ),
            )?;
        }
        let loaded = load_revisions(cache, o.law_type.as_deref())?;
        let (main, upcoming) = plan(loaded.revisions);
        if o.lint {
            let _ = fs::remove_dir_all(&lint_dir);
            fs::create_dir_all(&lint_dir)?;
        }
        let bills = Bills::load(cache)?;
        let history = History { main: &main, upcoming: &upcoming, bills: &bills };
        let streamed = stream(
            cache,
            &staging,
            &history,
            o.lint.then_some(lint_dir.as_path()),
            o.jobs,
            &previous,
        )?;
        git::run(&staging, &["reset", "-q", "--hard", "main"])?;
        // Loose and borrowed blobs → one delta-compressed pack, while lint runs on the same files.
        let mut repack = Command::new("git")
            .args(["repack", "-adq"])
            .current_dir(&staging)
            .spawn()?;
        let linted = if o.lint {
            Some(lint::run(&lint_dir, &o.rumdl))
        } else {
            None
        };
        if !repack.wait()?.success() {
            bail!("git repack failed");
        }
        fs::remove_file(&alternates).or_else(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Ok(())
            } else {
                Err(e)
            }
        })?;
        let mut report = Report {
            commits_main: main.len(),
            commits_upcoming: upcoming.len(),
            files: streamed.files,
            rendered: streamed
                .entries
                .iter()
                .filter(|(rid, e)| previous.get(*rid) != Some(e))
                .count(),
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
        git::copy_remote_config(out, &staging)?;
        clear(out)?;
        fs::remove_dir(out)?;
        fs::rename(&staging, out)?;
        if o.lint {
            // entries from a --no-lint build were never linted, so they must not skip lint later
            reuse::save(cache, &version, &streamed.entries)?;
        }
        Ok(report)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    let _ = fs::remove_dir_all(&lint_dir);
    let report = result?;
    fs::create_dir_all(&cache.root)?;
    fs::write(
        cache.root.join("build-report.json"),
        serde_json::to_string_pretty(&report.to_json(true))?,
    )?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{fixture_cache, LAW_ID};

    fn sh(repo: &Path, args: &[&str]) -> String {
        git::run(repo, args).unwrap()
    }

    fn opts(lint: bool, jobs: usize) -> Options {
        Options {
            lint,
            jobs,
            ..Default::default()
        }
    }

    #[test]
    fn builds_history_from_fixture() {
        let (d, cache) = fixture_cache();
        let out = d.path().join("out");
        let report = build(&cache, &out, &opts(false, 2)).unwrap();
        let subjects = sh(
            &out,
            &[
                "log",
                "--topo-order",
                "--format=%ad %s",
                "--date=short",
                "main",
            ],
        );
        let subjects: Vec<&str> = subjects.lines().collect();
        assert_eq!(subjects.len(), 4); // root + 3 amendments
        assert!(subjects[3].ends_with("リポジトリの初期化"));
        assert_eq!(
            subjects[..3]
                .iter()
                .map(|s| s.split(' ').next().unwrap())
                .collect::<Vec<_>>(),
            ["2020-01-01", "2019-05-01", "2016-01-01"]
        );
        let md = sh(&out, &["show", "main:laws/act/323/323AC1000000178.md"]);
        assert!(
            md.starts_with("---\nlaw_id: \"323AC1000000178\"\n")
                && md.contains("# 国民の祝日に関する法律")
        );
        assert!(
            report.commits_main == 3 && report.excluded.is_empty() && report.skipped.is_empty()
        );
        assert!(git::run(&out, &["rev-parse", "--verify", "-q", "upcoming"]).is_err());
        assert!(out.join("laws/act/323/323AC1000000178.md").exists()); // working tree checked out
    }

    #[test]
    fn build_is_deterministic() {
        let (d, cache) = fixture_cache();
        build(&cache, &d.path().join("a"), &opts(false, 1)).unwrap();
        build(&cache, &d.path().join("b"), &opts(false, 4)).unwrap();
        assert_eq!(
            sh(&d.path().join("a"), &["rev-parse", "main"]),
            sh(&d.path().join("b"), &["rev-parse", "main"])
        );
    }

    #[test]
    fn refuses_unknown_nonempty_dir() {
        let (d, cache) = fixture_cache();
        let out = d.path().join("out");
        fs::create_dir(&out).unwrap();
        fs::write(out.join("precious.txt"), "keep").unwrap();
        assert!(build(&cache, &out, &opts(false, 1))
            .unwrap_err()
            .to_string()
            .contains("not a previous build"));
        assert_eq!(
            fs::read_to_string(out.join("precious.txt")).unwrap(),
            "keep"
        );
    }

    #[test]
    fn rebuild_replaces_previous_build_and_keeps_remote_config() {
        let (d, cache) = fixture_cache();
        let out = d.path().join("out");
        build(&cache, &out, &opts(false, 1)).unwrap();
        sh(
            &out,
            &["remote", "add", "origin", "git@example.com:x/houritsu.git"],
        );
        sh(&out, &["config", "branch.main.remote", "origin"]);
        build(&cache, &out, &opts(false, 1)).unwrap();
        assert_eq!(sh(&out, &["log", "--format=%s", "main"]).lines().count(), 4);
        assert_eq!(
            sh(&out, &["remote", "get-url", "origin"]),
            "git@example.com:x/houritsu.git\n"
        );
        assert_eq!(sh(&out, &["config", "branch.main.remote"]), "origin\n");
    }

    #[test]
    fn rumdl_passes_on_every_revision() {
        let (d, cache) = fixture_cache();
        assert_eq!(
            build(&cache, &d.path().join("out"), &opts(true, 2))
                .unwrap()
                .lint,
            "ok"
        );
    }

    #[test]
    fn lint_failure_keeps_previous_build_and_leaves_no_staging() {
        let (d, cache) = fixture_cache();
        let out = d.path().join("out");
        build(&cache, &out, &opts(false, 1)).unwrap();
        let before = sh(&out, &["rev-parse", "main"]);
        let fake = [
            "sh",
            "-c",
            "head -c 5000 /dev/zero | tr '\\0' x; exit 1",
            "rumdl",
        ];
        let o = Options {
            rumdl: fake.iter().map(|s| s.to_string()).collect(),
            ..opts(true, 1)
        };
        assert!(build(&cache, &out, &o)
            .unwrap_err()
            .to_string()
            .contains("rumdl failed"));
        assert_eq!(sh(&out, &["rev-parse", "main"]), before);
        assert!(!d.path().join("out.building").exists() && !cache.root.join("lint").exists());
        assert_eq!(
            fs::read_to_string(cache.root.join("lint.log"))
                .unwrap()
                .len(),
            5000
        );
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
        let data = cache
            .read(&cache.data_path(last["law_revision_id"].as_str().unwrap()))
            .unwrap();
        cache
            .write(
                &cache.data_path(&format!("{LAW_ID}_20300101_999AC0000000001")),
                &data,
            )
            .unwrap();
        let out = d.path().join("out");
        assert_eq!(
            build(&cache, &out, &opts(false, 1))
                .unwrap()
                .commits_upcoming,
            1
        );
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
        assert_eq!(
            build(&cache, &out, &opts(false, 1)).unwrap().commits_main,
            4
        );
        assert_eq!(
            sh(&out, &["log", "-1", "--format=%s", "main"]),
            "国民の祝日に関する法律を廃止する法律 施行\n"
        );
        assert_eq!(
            sh(&out, &["ls-tree", "-r", "--name-only", "main", "laws"]),
            ""
        );
        assert!(!out.join(format!("laws/act/323/{LAW_ID}.md")).exists());
        assert_eq!(
            sh(
                &out,
                &[
                    "log",
                    "--format=%h",
                    "main",
                    "--",
                    &format!("laws/act/323/{LAW_ID}.md")
                ]
            )
            .split_whitespace()
            .count(),
            4
        );
    }

    #[test]
    fn worker_error_propagates_and_cleans_up() {
        let (d, cache) = fixture_cache();
        let rid = cache.read(&cache.revisions_path(LAW_ID)).unwrap()["revisions"][0]
            ["law_revision_id"]
            .as_str()
            .unwrap()
            .to_string();
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
        let changed = Options {
            code_version: Some("changed".into()),
            ..opts(true, 1)
        };
        assert_eq!(build(&cache, &out, &changed).unwrap().rendered, first.files);
    }

    #[test]
    fn rebuild_renders_again_when_the_previous_build_is_gone() {
        let (d, cache) = fixture_cache();
        build(&cache, &d.path().join("a"), &opts(true, 1)).unwrap();
        assert!(
            build(&cache, &d.path().join("b"), &opts(true, 1))
                .unwrap()
                .rendered
                > 0
        );
    }

    #[test]
    fn no_lint_build_does_not_let_later_builds_skip_lint() {
        let (d, cache) = fixture_cache();
        let out = d.path().join("out");
        let first = build(&cache, &out, &opts(false, 1)).unwrap();
        assert_eq!(
            build(&cache, &out, &opts(true, 1)).unwrap().rendered,
            first.files
        );
    }

    #[test]
    fn rebuild_renders_a_revision_whose_data_changed() {
        let (d, cache) = fixture_cache();
        let out = d.path().join("out");
        build(&cache, &out, &opts(true, 1)).unwrap();
        let rid = cache.read(&cache.revisions_path(LAW_ID)).unwrap()["revisions"][0]
            ["law_revision_id"]
            .as_str()
            .unwrap()
            .to_string();
        let path = cache.data_path(&rid);
        let f = fs::File::options().write(true).open(&path).unwrap();
        f.set_modified(
            fs::metadata(&path).unwrap().modified().unwrap() + std::time::Duration::from_nanos(1),
        )
        .unwrap();
        assert_eq!(build(&cache, &out, &opts(true, 1)).unwrap().rendered, 1);
    }
}
