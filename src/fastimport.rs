use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use sha1::{Digest, Sha1};

pub const COMMITTER: &str = "houritsu-builder <houritsu-builder@users.noreply.github.com>";

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Midnight JST of a `YYYY-MM-DD` date as a Unix time; git rejects dates before 1970-01-02.
pub fn jst_midnight(date: &str) -> anyhow::Result<i64> {
    let parts: Vec<i64> = date
        .split('-')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .map_err(|_| anyhow::anyhow!("bad date {date:?}"))?;
    let [y, m, d] = parts[..] else {
        anyhow::bail!("bad date {date:?}")
    };
    Ok((days_from_civil(y, m, d) * 86400 - 9 * 3600).max(86400))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

static TMP: AtomicU64 = AtomicU64::new(0);

/// Stores data as a loose git blob, so compression runs in parallel workers instead of in fast-import.
pub fn write_blob(objects: &Path, data: &[u8]) -> anyhow::Result<String> {
    let mut raw = format!("blob {}\0", data.len()).into_bytes();
    raw.extend_from_slice(data);
    let sha = hex(&Sha1::digest(&raw));
    let path = objects.join(&sha[..2]).join(&sha[2..]);
    if !path.exists() {
        fs::create_dir_all(path.parent().unwrap())?;
        let tmp = path.with_file_name(format!(
            "{}.{}.{}.tmp",
            &sha[2..],
            std::process::id(),
            TMP.fetch_add(1, Ordering::Relaxed)
        ));
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(1));
        z.write_all(&raw)?;
        fs::write(&tmp, z.finish()?)?;
        fs::rename(&tmp, &path)?;
    }
    Ok(sha)
}

pub struct Stream<W: Write> {
    out: W,
    mark: u64,
    marks: HashMap<String, u64>,
}

#[derive(Default)]
pub struct Commit<'a> {
    pub ref_: &'a str,
    pub date: &'a str,
    pub message: &'a str,
    pub files: BTreeMap<String, String>,
    pub blobs: BTreeMap<String, String>,
    pub deleted: BTreeSet<String>,
    pub from_ref: Option<&'a str>,
    pub author: Option<&'a str>,
}

impl<W: Write> Stream<W> {
    pub fn new(out: W) -> Self {
        Stream {
            out,
            mark: 0,
            marks: HashMap::new(),
        }
    }

    fn data(&mut self, text: &str) -> std::io::Result<()> {
        write!(self.out, "data {}\n{text}\n", text.len())
    }

    pub fn commit(&mut self, c: Commit) -> anyhow::Result<u64> {
        self.mark += 1;
        let ts = jst_midnight(c.date)?;
        let author = match c.author.filter(|a| !a.is_empty()) {
            Some(a) => format!("{a} <>"),
            None => COMMITTER.to_string(),
        };
        write!(
            self.out,
            "commit {}\nmark :{}\nauthor {author} {ts} +0900\ncommitter {COMMITTER} {ts} +0900\n",
            c.ref_, self.mark
        )?;
        self.data(c.message)?;
        if let Some(from) = c.from_ref {
            match self.marks.get(from) {
                Some(m) => writeln!(self.out, "from :{m}")?,
                None => writeln!(self.out, "from {from}")?,
            }
        }
        for path in &c.deleted {
            writeln!(self.out, "D {path}")?;
        }
        let paths: BTreeSet<&String> = c.files.keys().chain(c.blobs.keys()).collect();
        for path in paths {
            match c.blobs.get(path) {
                Some(sha) => writeln!(self.out, "M 100644 {sha} {path}")?,
                None => {
                    writeln!(self.out, "M 100644 inline {path}")?;
                    self.data(&c.files[path])?;
                }
            }
        }
        writeln!(self.out)?;
        self.marks.insert(c.ref_.to_string(), self.mark);
        Ok(self.mark)
    }

    pub fn done(&mut self) -> std::io::Result<()> {
        self.out.write_all(b"done\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(repo: &Path, args: &[&str]) -> String {
        let o = Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8(o.stdout).unwrap()
    }

    fn import(repo: &Path, stream: &[u8]) {
        git(repo, &["init", "-q", "-b", "main", "."]);
        let mut p = Command::new("git")
            .args(["fast-import", "--quiet", "--done"])
            .current_dir(repo)
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        p.stdin.take().unwrap().write_all(stream).unwrap();
        assert!(p.wait().unwrap().success());
    }

    fn files(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn jst_midnight_values() {
        assert_eq!(jst_midnight("2017-03-01").unwrap(), 1488294000);
        assert_eq!(jst_midnight("1960-01-01").unwrap(), 86400);
    }

    #[test]
    fn stream_imports_into_git() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Stream::new(Vec::new());
        s.commit(Commit {
            ref_: "refs/heads/main",
            date: "2017-03-01",
            message: "初版\n",
            files: files(&[("laws/act/A.md", "# 法\n")]),
            author: Some("平成十五年法律第五十七号"),
            ..Default::default()
        })
        .unwrap();
        s.commit(Commit {
            ref_: "refs/heads/upcoming",
            date: "2027-01-17",
            message: "改正\n",
            files: files(&[("laws/act/A.md", "# 法　改\n")]),
            from_ref: Some("refs/heads/main"),
            author: Some("令和八年法律第五十六号"),
            ..Default::default()
        })
        .unwrap();
        s.done().unwrap();
        import(d.path(), &s.out);
        let log = git(
            d.path(),
            &[
                "log",
                "--format=%an|%ae|%cn|%ad|%s",
                "--date=iso",
                "upcoming",
            ],
        );
        assert_eq!(
            log.lines().collect::<Vec<_>>(),
            [
                "令和八年法律第五十六号||houritsu-builder|2027-01-17 00:00:00 +0900|改正",
                "平成十五年法律第五十七号||houritsu-builder|2017-03-01 00:00:00 +0900|初版"
            ]
        );
        assert_eq!(git(d.path(), &["show", "main:laws/act/A.md"]), "# 法\n");
    }

    #[test]
    fn stream_deletes_files() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Stream::new(Vec::new());
        s.commit(Commit {
            ref_: "refs/heads/main",
            date: "2017-03-01",
            message: "初版\n",
            files: files(&[("laws/act/A.md", "# 法\n"), ("laws/act/B.md", "# 乙\n")]),
            ..Default::default()
        })
        .unwrap();
        s.commit(Commit {
            ref_: "refs/heads/main",
            date: "2018-03-01",
            message: "廃止\n",
            deleted: ["laws/act/A.md", "laws/act/C.md"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            ..Default::default()
        })
        .unwrap();
        s.done().unwrap();
        import(d.path(), &s.out);
        assert_eq!(
            git(d.path(), &["ls-tree", "-r", "--name-only", "main"]),
            "laws/act/B.md\n"
        );
    }

    #[test]
    fn stream_references_loose_blobs() {
        let d = tempfile::tempdir().unwrap();
        git(d.path(), &["init", "-q", "-b", "main", "."]);
        let sha = write_blob(&d.path().join(".git/objects"), "# 法\n".as_bytes()).unwrap();
        let mut h = Command::new("git")
            .args(["hash-object", "--stdin"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        h.stdin
            .take()
            .unwrap()
            .write_all("# 法\n".as_bytes())
            .unwrap();
        assert_eq!(
            String::from_utf8(h.wait_with_output().unwrap().stdout)
                .unwrap()
                .trim(),
            sha
        );
        let mut s = Stream::new(Vec::new());
        s.commit(Commit {
            ref_: "refs/heads/main",
            date: "2017-03-01",
            message: "初版\n",
            files: files(&[("README.md", "r\n")]),
            blobs: files(&[("laws/act/A.md", &sha)]),
            ..Default::default()
        })
        .unwrap();
        s.done().unwrap();
        let mut p = Command::new("git")
            .args(["fast-import", "--quiet", "--done"])
            .current_dir(d.path())
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        p.stdin.take().unwrap().write_all(&s.out).unwrap();
        assert!(p.wait().unwrap().success());
        assert_eq!(git(d.path(), &["show", "main:laws/act/A.md"]), "# 法\n");
        git(d.path(), &["fsck", "--strict"]);
    }
}
