use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::anyhow;
use rumdl_lib::config::{Config, ConfigLoaded, MarkdownFlavor, SourcedConfig};
use rumdl_lib::rule::Rule;

/// The config shipped in houritsu, so readers can run the same check.
pub const CONFIG: &str = include_str!("../assets/rumdl.toml");

/// rumdl, pinned in Cargo.lock and run in-process on each rendered revision.
pub struct Linter {
    rules: Vec<Box<dyn Rule>>,
    config: Config,
    flavor: MarkdownFlavor,
}

impl Linter {
    pub fn new(config: &str) -> anyhow::Result<Self> {
        // rumdl parses its config from a file named `.rumdl.toml`.
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let dir =
            std::env::temp_dir().join(format!("houritsu-rumdl-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir)?;
        let path = dir.join(".rumdl.toml");
        fs::write(&path, config)?;
        let loaded = SourcedConfig::<ConfigLoaded>::load_config_for_path(&path, &dir);
        let _ = fs::remove_dir_all(&dir);
        let config = loaded.map_err(|e| anyhow!("rumdl config: {e}"))?;
        let rules =
            rumdl_lib::rules::filter_rules(&rumdl_lib::rules::all_rules(&config), &config.global);
        let flavor = config.markdown_flavor();
        Ok(Linter {
            rules,
            config,
            flavor,
        })
    }

    /// Warnings as `path:line:column: [rule] message`, like the rumdl CLI prints them.
    pub fn check(&self, path: &str, text: &str) -> anyhow::Result<Vec<String>> {
        let warnings = rumdl_lib::lint(
            text,
            &self.rules,
            false,
            self.flavor,
            Some(PathBuf::from(path)),
            Some(&self.config),
        )
        .map_err(|e| anyhow!("rumdl on {path}: {e}"))?;
        Ok(warnings
            .iter()
            .map(|w| {
                format!(
                    "{path}:{}:{}: [{}] {}",
                    w.line,
                    w.column,
                    w.rule_name.as_deref().unwrap_or("?"),
                    w.message
                )
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::render_law;

    #[test]
    fn table_fixture_passes_shipped_config() {
        let xml = crate::testutil::fixture_xmls().pop().unwrap();
        let warnings = Linter::new(CONFIG)
            .unwrap()
            .check("x.md", &render_law(&xml).unwrap().markdown)
            .unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn shipped_config_rejects_a_skipped_heading_level() {
        let warnings = Linter::new(CONFIG)
            .unwrap()
            .check("x.md", "# 法\n\n### 第一条\n\n本文\n")
            .unwrap();
        assert!(
            warnings.iter().any(|w| w.starts_with("x.md:3:1: [MD001]")),
            "{warnings:?}"
        );
    }

    #[test]
    fn shipped_config_allows_what_statute_text_needs() {
        // a lone U+3000 line, a tab, a long line, runs of spaces, full-width punctuation in a heading
        let text = "# 法\n\n## 第一条（目的）。\n\n　\n\nあ\tい  う".to_string()
            + &"長".repeat(300)
            + "\n";
        assert_eq!(
            Linter::new(CONFIG).unwrap().check("x.md", &text).unwrap(),
            Vec::<String>::new()
        );
    }
}
