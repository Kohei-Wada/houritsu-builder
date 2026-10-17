use std::collections::HashSet;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::bail;

pub fn run(dir: &Path, args: &[&str]) -> anyhow::Result<String> {
    let out = Command::new("git").args(args).current_dir(dir).stderr(Stdio::inherit()).output()?;
    if !out.status.success() {
        bail!("git {} failed in {}", args.join(" "), dir.display());
    }
    Ok(String::from_utf8(out.stdout)?)
}

/// The objects of `shas` that the repository has.
pub fn present<'a>(repo: &Path, shas: impl Iterator<Item = &'a str>) -> anyhow::Result<HashSet<String>> {
    let mut p = Command::new("git")
        .args(["cat-file", "--batch-check=%(objectname)"])
        .current_dir(repo)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let input: String = shas.map(|s| format!("{s}\n")).collect();
    let mut stdin = p.stdin.take().unwrap();
    let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let output = p.wait_with_output()?;
    writer.join().unwrap()?;
    Ok(std::str::from_utf8(&output.stdout)?.lines().filter(|l| !l.ends_with(" missing")).map(str::to_string).collect())
}

/// Carries remotes and branch tracking over to a rebuilt repository, so pushing keeps working.
pub fn copy_remote_config(old: &Path, new: &Path) -> anyhow::Result<()> {
    if !old.join(".git").exists() {
        return Ok(());
    }
    let found = Command::new("git").args(["config", "--local", "--get-regexp", r"^(remote|branch)\."]).current_dir(old).output()?;
    for line in String::from_utf8(found.stdout)?.lines() {
        if let Some((key, value)) = line.split_once(' ') {
            run(new, &["config", "--add", key, value])?;
        }
    }
    Ok(())
}
