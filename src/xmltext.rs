//! Text access on roxmltree nodes with ElementTree's semantics, which the
//! output format was defined against: `text` is the text before the first child
//! element, `tail` the text after an element, comments and processing
//! instructions are dropped and the text around them joins.

use roxmltree::Node;

use crate::render::inline_figure;

pub type El<'a, 'i> = Node<'a, 'i>;

pub fn tag<'a>(el: El<'a, '_>) -> &'a str {
    el.tag_name().name()
}

pub fn elements<'a, 'i>(el: El<'a, 'i>) -> impl Iterator<Item = El<'a, 'i>> {
    el.children().filter(|c| c.is_element())
}

pub fn find<'a, 'i>(el: El<'a, 'i>, name: &str) -> Option<El<'a, 'i>> {
    elements(el).find(|c| tag(*c) == name)
}

fn texts_until_element<'a, 'i>(mut node: Option<El<'a, 'i>>, out: &mut String) {
    while let Some(n) = node {
        if n.is_element() {
            break;
        }
        if n.is_text() {
            out.push_str(n.text().unwrap_or(""));
        }
        node = n.next_sibling();
    }
}

pub fn push_text(el: El, out: &mut String) {
    texts_until_element(el.first_child(), out);
}

pub fn push_tail(el: El, out: &mut String) {
    texts_until_element(el.next_sibling(), out);
}

pub fn text(el: El) -> String {
    let mut s = String::new();
    push_text(el, &mut s);
    s
}

/// Removes the pretty-printing indentation e-Gov puts between elements, but not
/// the ideographic spaces that are part of the statute text.
pub fn strip_pretty(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\n' {
            while matches!(chars.peek(), Some(' ' | '\t')) {
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out.trim_matches([' ', '\t', '\r', '\n']).to_string()
}

pub fn retain_non_ascii_ws(s: &mut String) {
    s.retain(|c| !matches!(c, ' ' | '\t' | '\r' | '\n'));
}

fn push_inline(el: El, revision: Option<&str>, out: &mut String) {
    push_text(el, out);
    for child in elements(el) {
        match tag(child) {
            "Ruby" => {
                out.push_str("<ruby>");
                push_text(child, out);
                for c in elements(child) {
                    if tag(c) == "Rt" {
                        out.push_str("<rt>");
                        push_inline(c, revision, out);
                        out.push_str("</rt>");
                    } else {
                        push_inline(c, revision, out);
                    }
                    push_tail(c, out);
                }
                out.push_str("</ruby>");
            }
            t @ ("Sup" | "Sub") => {
                let t = t.to_lowercase();
                out.push_str(&format!("<{t}>"));
                push_inline(child, revision, out);
                out.push_str(&format!("</{t}>"));
            }
            "Fig" => {
                if let Some(rid) = revision {
                    out.push_str(&inline_figure(rid, child.attribute("src").unwrap_or("")));
                }
            }
            _ => push_inline(child, revision, out),
        }
        push_tail(child, out);
    }
}

/// The text of an element with its markup as HTML; with a revision ID, figures in the text
/// link to that revision's attachments.
pub fn inline(el: El, revision: Option<&str>) -> String {
    let mut s = String::new();
    push_inline(el, revision, &mut s);
    strip_pretty(&s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inline_of(xml: &str) -> String {
        let doc = roxmltree::Document::parse(xml).unwrap();
        inline(doc.root_element(), None)
    }

    #[test]
    fn strip_pretty_keeps_ideographic_space() {
        assert_eq!(strip_pretty("\n      附　則\n    "), "附　則");
    }

    #[test]
    fn ascii_ws_removal_keeps_ideographic_space() {
        let mut s = " a\tb\nc　d ".to_string();
        retain_non_ascii_ws(&mut s);
        assert_eq!(s, "abc　d");
    }

    #[test]
    fn inline_joins_sentences_without_pretty_whitespace() {
        assert_eq!(
            inline_of("<S>\n  <Sentence>一。</Sentence>\n  <Sentence>二。</Sentence>\n</S>"),
            "一。二。"
        );
    }

    #[test]
    fn inline_ruby() {
        assert_eq!(
            inline_of("<Sentence><Ruby>𠮟<Rt>しつ</Rt></Ruby>責する。</Sentence>"),
            "<ruby>𠮟<rt>しつ</rt></ruby>責する。"
        );
    }

    #[test]
    fn inline_ruby_multiple_rt_keeps_order() {
        assert_eq!(
            inline_of("<S><Ruby>漢<Rt>かん</Rt>字<Rt>じ</Rt></Ruby></S>"),
            "<ruby>漢<rt>かん</rt>字<rt>じ</rt></ruby>"
        );
    }

    #[test]
    fn inline_sup_sub() {
        assert_eq!(
            inline_of("<Sentence>Ａ<Sup>２</Sup>、Ｂ<Sub>１</Sub>及び</Sentence>"),
            "Ａ<sup>２</sup>、Ｂ<sub>１</sub>及び"
        );
    }

    #[test]
    fn comments_are_dropped_and_text_around_them_joins() {
        assert_eq!(
            inline_of("<S>前<!-- c -->後<A/>尾<!-- d -->端</S>"),
            "前後尾端"
        );
    }
}
