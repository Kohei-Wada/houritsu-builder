//! Rendering one revision to its blob. Everything here decides a blob's bytes or its lint
//! result, so it is part of the code version that `rendered.json` is keyed on.

use std::path::Path;

use anyhow::Context;
use base64::Engine;
use rayon::prelude::*;

use crate::cache::Cache;
use crate::fastimport::write_blob;
use crate::fidelity::check_doc;
use crate::frontmatter::frontmatter;
use crate::history::Revision;
use crate::lint::Linter;
use crate::render::{parse, render_doc, Counts};

pub struct Rendered {
    pub sha: String,
    pub unknown: Counts,
    pub warnings: Vec<String>,
}

fn render(
    cache: &Cache,
    objects: &Path,
    r: &Revision,
    linter: Option<&Linter>,
) -> anyhow::Result<Rendered> {
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
    let rendered =
        render_doc(&doc, Some(r.revision_id())).with_context(|| r.revision_id().to_string())?;
    check_doc(&doc, &rendered.markdown, r.revision_id())?;
    let text = frontmatter(&r.law_info, &r.rev) + &rendered.markdown;
    let warnings = match linter {
        Some(l) => l.check(
            &format!(
                "{}/{}.md",
                r.path().trim_end_matches(".md"),
                r.revision_id()
            ),
            &text,
        )?,
        None => vec![],
    };
    Ok(Rendered {
        sha: write_blob(objects, text.as_bytes())?,
        unknown: rendered.unknown,
        warnings,
    })
}

pub fn render_all(
    cache: &Cache,
    objects: &Path,
    todo: &[&Revision],
    linter: Option<&Linter>,
    jobs: usize,
) -> anyhow::Result<Vec<Rendered>> {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs.max(1))
        .build()?;
    pool.install(|| {
        todo.par_iter()
            .map(|r| render(cache, objects, r, linter))
            .collect()
    })
}
