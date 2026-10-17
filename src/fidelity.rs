//! The statute text is never changed: the text of the XML must equal the text of
//! the generated markdown, character for character, ignoring ASCII whitespace.

use std::sync::LazyLock;

use regex::Regex;

use crate::render::{
    law_body, parse, suppl_source, ATTACHMENT, CONTAINERS, NO_IMAGE, SKIPPED_TAGS,
};
use crate::xmltext::{elements, push_tail, push_text, retain_non_ascii_ws, tag, El};

// Rare (after an escaping backslash); the per-line work below avoids Regex, whose shared cache serialises threads.
static DIGIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d").unwrap());

#[derive(Debug)]
pub struct FidelityError(pub String);

impl std::fmt::Display for FidelityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for FidelityError {}

fn article_parent(t: &str) -> bool {
    t == "MainProvision" || t == "SupplProvision" || CONTAINERS.contains(&t)
}

fn walk(el: El, out: &mut String, parent: &str, quoted: bool) {
    let t = tag(el);
    let quoted = quoted || t == "AmendProvision"; // rendered as plain text in document order
    let mut children: Vec<El> = elements(el).collect();
    if t == "Article" && article_parent(parent) && !quoted {
        // rendered as title then caption
        if let Some(i) = children.iter().position(|c| tag(*c) == "ArticleCaption") {
            let caption = children.remove(i);
            let at = children
                .iter()
                .position(|c| tag(*c) == "ArticleTitle")
                .map_or(0, |i| i + 1);
            children.insert(at, caption);
        }
    }
    push_text(el, out);
    // the 附則 heading of the body also names its amending law (render::suppl_source)
    let source = if t == "SupplProvision" && parent.is_empty() {
        suppl_source(el)
    } else {
        String::new()
    };
    let has_label = children.iter().any(|c| tag(*c) == "SupplProvisionLabel");
    if !has_label {
        out.push_str(&source);
    }
    for c in children {
        walk(c, out, t, quoted);
        if tag(c) == "SupplProvisionLabel" {
            out.push_str(&source);
        }
        push_tail(c, out);
    }
}

pub fn xml_text(xml: &str) -> anyhow::Result<String> {
    doc_text(&parse(xml)?)
}

pub fn doc_text(doc: &roxmltree::Document) -> anyhow::Result<String> {
    let body = law_body(doc)?;
    let mut out = String::with_capacity(doc.input_text().len() / 2);
    push_text(body, &mut out);
    for c in elements(body) {
        if !SKIPPED_TAGS.contains(&tag(c)) {
            walk(c, &mut out, "", false);
        }
        push_tail(c, &mut out);
    }
    retain_non_ascii_ws(&mut out);
    Ok(out)
}

fn without_frontmatter(md: &str) -> &str {
    if let Some(rest) = md.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---\n") {
            return &rest[end + 5..];
        }
    }
    md
}

/// Drops the backslash that `render::literal` put before markup at the start of a line, and the
/// indentation before it, which the comparison ignores anyway.
fn unescape(line: &str) -> &str {
    let rest = line.trim_start_matches(' ');
    if let Some(after) = rest.strip_prefix('\\') {
        if after.starts_with(['-', '+', '*', '#', '>', '=']) || DIGIT.is_match(after) {
            return after;
        }
    }
    line
}

/// Strips a heading marker (`#`..`######` and a space) or a list marker (`- `, or a bare `-`).
fn strip_prefix(line: &str) -> &str {
    let hashes = line.len() - line.trim_start_matches('#').len();
    if (1..=6).contains(&hashes) {
        return line[hashes..].strip_prefix(' ').unwrap_or(line);
    }
    let rest = line.trim_start_matches(' ');
    match rest.strip_prefix('-') {
        Some(after) if after.starts_with(' ') => &after[1..],
        Some("") => "",
        _ => line,
    }
}

/// The HTML the renderer writes: ruby and super/subscripts in text, and tables.
const TAGS: [&str; 9] = ["ruby", "rt", "sup", "sub", "table", "tr", "th", "td", "br"];

/// The length of a renderer-written tag at the start of `s`, attributes included.
fn tag_len(s: &str) -> Option<usize> {
    let name = s.strip_prefix('<')?;
    let name = name.strip_prefix('/').unwrap_or(name);
    let t = TAGS
        .iter()
        .find(|t| name.starts_with(**t) && name[t.len()..].starts_with(['>', ' ']))?;
    let after = &name[t.len()..];
    Some(s.len() - after.len() + after.find('>')? + 1)
}

fn push_without_tags(line: &str, out: &mut String) {
    let mut rest = line;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        match tag_len(rest) {
            Some(n) => rest = &rest[n..],
            None => {
                out.push('<');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
}

/// A line the renderer wrote for a figure (a link to its e-Gov attachment, or the mark for a
/// missing one), not statute text.
fn is_figure_line(line: &str) -> bool {
    let t = line.trim_start_matches(' ');
    t == NO_IMAGE
        || (t.starts_with('[') && t.ends_with(')') && t.contains(&format!("]({ATTACHMENT}")))
}

pub fn md_text(md: &str) -> String {
    let mut joined = String::with_capacity(md.len());
    for line in without_frontmatter(md).split('\n') {
        if !is_figure_line(line) {
            push_without_tags(unescape(strip_prefix(line)), &mut joined);
        }
    }
    retain_non_ascii_ws(&mut joined);
    joined
}

pub fn check(xml: &str, md: &str, label: &str) -> anyhow::Result<()> {
    check_doc(&parse(xml)?, md, label)
}

pub fn check_doc(doc: &roxmltree::Document, md: &str, label: &str) -> anyhow::Result<()> {
    let (a, b) = (doc_text(doc)?, md_text(md));
    if a == b {
        return Ok(());
    }
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let i = a
        .iter()
        .zip(&b)
        .position(|(x, y)| x != y)
        .unwrap_or(a.len().min(b.len()));
    let around = |s: &[char]| -> String {
        s[i.saturating_sub(20).min(s.len())..(i + 20).min(s.len())]
            .iter()
            .collect()
    };
    Err(FidelityError(format!(
        "{label}: text mismatch at char {i}: xml={:?} md={:?}",
        around(&a),
        around(&b)
    ))
    .into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::render_law;
    use crate::testutil::{fixture_xml, SUFFIXES};

    fn md(xml: &str) -> String {
        render_law(xml).unwrap().markdown
    }

    #[test]
    fn prefixes_match_the_original_regex() {
        for (line, want) in [
            ("# 法", "法"),
            ("###### 条", "条"),
            ("####### 条", "####### 条"),
            ("#法", "#法"),
            ("- 一", "一"),
            ("    - 二", "二"),
            ("-", ""),
            ("  -", ""),
            ("-x", "-x"),
            ("本文", "本文"),
            ("", ""),
        ] {
            assert_eq!(strip_prefix(line), want, "{line:?}");
        }
        let mut out = String::new();
        push_without_tags("a<ruby>漢<rt>かん</rt></ruby><b>x</sup>", &mut out);
        assert_eq!(out, "a漢かん<b>x");
        let mut out = String::new();
        push_without_tags(
            "<tr><td rowspan=\"2\">甲<br>乙</td><th>丙</th></tr><trx><tdd>",
            &mut out,
        );
        assert_eq!(out, "甲乙丙<trx><tdd>");
    }

    #[test]
    fn real_fixtures_pass() {
        for suffix in SUFFIXES {
            let xml = fixture_xml(suffix);
            check(&xml, &md(&xml), suffix).unwrap();
        }
    }

    #[test]
    fn dropped_character_fails() {
        let xml = fixture_xml(SUFFIXES[2]);
        let mutated = md(&xml).replacen("元日", "元", 1);
        assert!(check(&xml, &mutated, "mutated")
            .unwrap_err()
            .to_string()
            .contains("mismatch at char"));
    }

    #[test]
    fn ascii_markup_in_text_survives() {
        let xml = "<Law><LawBody><LawTitle>法</LawTitle><MainProvision><Paragraph><ParagraphNum/><ParagraphSentence><Sentence>&lt;b&gt;*注*_x_</Sentence></ParagraphSentence></Paragraph></MainProvision></LawBody></Law>";
        let m = md(xml);
        assert!(m.contains("<b>*注*_x_"));
        check(xml, &m, "ascii").unwrap();
    }

    #[test]
    fn article_caption_order_and_frontmatter_ignored() {
        let xml = "<Law><LawBody><LawTitle>法</LawTitle><MainProvision><Article><ArticleCaption>（目的）</ArticleCaption><ArticleTitle>第一条</ArticleTitle><Paragraph><ParagraphNum/><ParagraphSentence><Sentence>本文。</Sentence></ParagraphSentence></Paragraph></Article></MainProvision></LawBody></Law>";
        assert_eq!(xml_text(xml).unwrap(), "法第一条（目的）本文。");
        assert_eq!(
            md_text(&format!("---\ntitle: \"x\"\n---\n\n{}", md(xml))),
            "法第一条（目的）本文。"
        );
    }

    #[test]
    fn sup_sub_tags_are_ignored() {
        let xml = "<Law><LawBody><MainProvision><Paragraph><ParagraphSentence><Sentence>A<Sup>2</Sup>B<Sub>1</Sub></Sentence></ParagraphSentence></Paragraph></MainProvision></LawBody></Law>";
        check(xml, "A<sup>2</sup>B<sub>1</sub>\n", "t").unwrap();
        assert!(check(xml, "A<sup>3</sup>B<sub>1</sub>\n", "t").is_err());
    }

    #[test]
    fn toc_quoted_in_a_sentence_is_text_but_top_level_toc_is_skipped() {
        let xml = "<Law><LawBody><LawTitle>法</LawTitle><TOC><TOCLabel>目次</TOCLabel></TOC><MainProvision><Paragraph><ParagraphNum/>\
            <ParagraphSentence><Sentence>「<QuoteStruct><TOC><TOCChapter><ChapterTitle>第一章　総則</ChapterTitle></TOCChapter></TOC>\
            </QuoteStruct>」に改める。</Sentence></ParagraphSentence></Paragraph></MainProvision></LawBody></Law>";
        assert_eq!(xml_text(xml).unwrap(), "法「第一章　総則」に改める。");
        check(xml, &md(xml), "t").unwrap();
    }

    #[test]
    fn quoted_article_keeps_document_order() {
        let xml = "<Law><LawBody><MainProvision><Paragraph><ParagraphNum/><ParagraphSentence><Sentence>次の一条を加える。\
            <QuoteStruct><Article><ArticleCaption>（目的）</ArticleCaption><ArticleTitle>第二条</ArticleTitle></Article></QuoteStruct>\
            </Sentence></ParagraphSentence></Paragraph></MainProvision></LawBody></Law>";
        assert_eq!(xml_text(xml).unwrap(), "次の一条を加える。（目的）第二条");
        check(xml, &md(xml), "t").unwrap();
    }

    #[test]
    fn empty_item_marker_is_not_text() {
        let xml = "<Law><LawBody><MainProvision><Paragraph><ParagraphNum/><ParagraphSentence><Sentence>本文。</Sentence></ParagraphSentence>\
            <Item><ItemTitle/><ItemSentence><Sentence/></ItemSentence></Item>\
            <Item><ItemTitle>三</ItemTitle><ItemSentence><Sentence>号</Sentence></ItemSentence></Item>\
            </Paragraph></MainProvision></LawBody></Law>";
        check(xml, &md(xml), "t").unwrap();
    }

    #[test]
    fn markdown_markers_in_statute_text_are_escaped() {
        let remarks: String = ["-", "# 注", "1. 一", "=", "> 引用"]
            .iter()
            .map(|t| format!("<Remarks><Sentence>{t}</Sentence></Remarks>"))
            .collect();
        let xml = format!(
            "<Law><LawBody><MainProvision><Paragraph><ParagraphNum/><ParagraphSentence><Sentence>本文。</Sentence></ParagraphSentence>\
             <Item><ItemSentence><Sentence>- 項目</Sentence></ItemSentence><TableStruct>{remarks}<Table><TableRow>\
             <TableColumn><Sentence># 表</Sentence></TableColumn></TableRow></Table></TableStruct></Item>\
             </Paragraph></MainProvision></LawBody></Law>"
        );
        let m = md(&xml);
        assert!(
            m.contains("\n\n  \\-\n\n")
                && m.contains("- \\- 項目")
                && m.contains("  \\# 注")
                && m.contains("\\1. 一")
                && m.contains("  <td># 表</td>"), // markdown is not parsed inside HTML
            "{m}"
        );
        check(&xml, &m, "x").unwrap();
    }

    #[test]
    fn article_in_an_amend_provision_keeps_document_order_inside_a_chapter() {
        let xml = "<Law><LawBody><SupplProvision><Paragraph><ParagraphNum/><ParagraphSentence><Sentence>改める。</Sentence></ParagraphSentence>\
            <AmendProvision><NewProvision><Chapter><ChapterTitle>第七章の二</ChapterTitle><Article><ArticleCaption>（基準）</ArticleCaption>\
            <ArticleTitle>第百三十六条の二</ArticleTitle></Article></Chapter></NewProvision></AmendProvision>\
            </Paragraph></SupplProvision></LawBody></Law>";
        check(xml, &md(xml), "x").unwrap();
    }
}
