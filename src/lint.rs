use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::Context;

pub const RUMDL: [&str; 2] = ["uvx", "rumdl@0.2.79"];
pub const CONFIG: &str = include_str!("../assets/rumdl.toml");

/// Runs rumdl with the shipped config over a directory; returns whether it passed and its output.
pub fn run(dir: &Path, rumdl: &[String]) -> anyhow::Result<(bool, String)> {
    fs::create_dir_all(dir)?;
    fs::write(dir.join(".rumdl.toml"), CONFIG)?;
    let out = Command::new(&rumdl[0])
        .args(&rumdl[1..])
        .args(["check", "--no-cache", "--respect-gitignore=false", "."])
        .current_dir(dir)
        .output()
        .with_context(|| format!("running {}", rumdl.join(" ")))?;
    Ok((out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::render_law;

    fn rumdl() -> Vec<String> {
        RUMDL.iter().map(|s| s.to_string()).collect()
    }

    fn lint_one(markdown: &str) -> (bool, String) {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("laws/act/x.md");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, markdown).unwrap();
        run(d.path(), &rumdl()).unwrap()
    }

    #[test]
    fn table_fixture_passes_shipped_rumdl_config() {
        let xml = crate::testutil::fixture_xmls().pop().unwrap();
        let (ok, output) = lint_one(&render_law(&xml).unwrap().markdown);
        assert!(ok, "{output}");
    }

    #[test]
    fn shipped_rumdl_config_rejects_a_skipped_heading_level() {
        let (ok, output) = lint_one("# 法\n\n### 第一条\n\n本文\n");
        assert!(!ok && output.contains("MD001"), "{output}");
    }
}
