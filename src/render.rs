use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;

use crate::xmltext::{elements, find, inline, tag, El};

pub const CONTAINERS: [&str; 5] = ["Part", "Chapter", "Section", "Subsection", "Division"];
pub const SKIPPED_TAGS: [&str; 1] = ["TOC"];

pub type Counts = BTreeMap<String, u64>;

// Only for the rare text that starts with a digit: a Regex shared by every thread serialises them on its cache.
static DIGITS_THEN_MARK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d+[.)]").unwrap());

const ITEMS: [&str; 11] = [
    "Item",
    "Subitem1",
    "Subitem2",
    "Subitem3",
    "Subitem4",
    "Subitem5",
    "Subitem6",
    "Subitem7",
    "Subitem8",
    "Subitem9",
    "Subitem10",
];

fn is_item(t: &str) -> bool {
    ITEMS.contains(&t)
}

/// Escapes statute text that markdown would read as a list, heading, quote or setext underline.
pub fn literal(text: &str) -> String {
    let markup = match text.chars().next() {
        Some('-' | '+' | '*' | '#' | '>' | '=') => true,
        Some(c) if c.is_numeric() => DIGITS_THEN_MARK.is_match(text),
        _ => false,
    };
    if markup {
        format!("\\{text}")
    } else {
        text.to_string()
    }
}

pub struct Rendered {
    pub markdown: String,
    pub unknown: Counts,
}

#[derive(Default)]
struct Renderer {
    blocks: Vec<String>,
    unknown: Counts,
}

fn same(a: Option<El>, b: El) -> bool {
    a == Some(b)
}

impl Renderer {
    fn block(&mut self, text: String) {
        if !text.is_empty() {
            self.blocks.push(text);
        }
    }

    fn heading(&mut self, level: usize, text: String) {
        if !text.is_empty() {
            self.block(format!("{} {text}", "#".repeat(level.min(6))));
        }
    }

    fn fallback(&mut self, el: El, indent: &str) {
        *self.unknown.entry(tag(el).to_string()).or_default() += 1;
        let text = inline(el);
        if !text.is_empty() {
            self.block(format!("{indent}{}", literal(&text)));
        }
    }

    fn body(&mut self, body: El) {
        for el in elements(body) {
            match tag(el) {
                "LawTitle" => self.heading(1, inline(el)),
                t if SKIPPED_TAGS.contains(&t) => {}
                "EnactStatement" => self.block(literal(&inline(el))),
                "Preamble" => {
                    for p in elements(el) {
                        if tag(p) == "Paragraph" {
                            self.paragraph(p);
                        } else {
                            self.fallback(p, "");
                        }
                    }
                }
                "MainProvision" => self.provision(elements(el).collect(), 2),
                "SupplProvision" => self.suppl(el),
                t if t.starts_with("Appdx") => self.appendix(el, 2),
                _ => self.fallback(el, ""),
            }
        }
    }

    fn provision(&mut self, children: Vec<El>, level: usize) {
        for child in children {
            match tag(child) {
                t if CONTAINERS.contains(&t) => self.container(child, level),
                "Article" => self.article(child, level),
                "Paragraph" => self.paragraph(child),
                _ => self.fallback(child, ""),
            }
        }
    }

    fn container(&mut self, el: El, level: usize) {
        let title = find(el, &format!("{}Title", tag(el)));
        self.heading(level, title.map(inline).unwrap_or_default());
        self.provision(
            elements(el).filter(|c| !same(title, *c)).collect(),
            level + 1,
        );
    }

    fn article(&mut self, el: El, level: usize) {
        let (title, caption) = (find(el, "ArticleTitle"), find(el, "ArticleCaption"));
        self.heading(
            level,
            title.map(inline).unwrap_or_default() + &caption.map(inline).unwrap_or_default(),
        );
        for child in elements(el) {
            if same(title, child) || same(caption, child) {
                continue;
            }
            if tag(child) == "Paragraph" {
                self.paragraph(child);
            } else {
                self.fallback(child, "");
            }
        }
    }

    fn sentence(&self, el: Option<El>) -> String {
        match el {
            None => String::new(),
            Some(el) if find(el, "Column").is_some() => {
                elements(el).map(inline).collect::<Vec<_>>().join(" ")
            }
            Some(el) => inline(el),
        }
    }

    fn paragraph(&mut self, el: El) {
        let (caption, num, sentence) = (
            find(el, "ParagraphCaption"),
            find(el, "ParagraphNum"),
            find(el, "ParagraphSentence"),
        );
        if let Some(c) = caption {
            self.block(literal(&inline(c)));
        }
        let n = num.map(inline).unwrap_or_default();
        let text = [n, self.sentence(sentence)]
            .into_iter()
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        self.block(literal(&text));
        for child in elements(el) {
            if same(caption, child) || same(num, child) || same(sentence, child) {
                continue;
            }
            match tag(child) {
                t if is_item(t) => self.item(child, 0),
                "TableStruct" => self.table(child, ""),
                "List" => self.list(child, 0),
                "FigStruct" => self.figstruct(child),
                _ => self.fallback(child, ""),
            }
        }
    }

    fn item(&mut self, el: El, depth: usize) {
        let t = tag(el);
        let (title, sentence) = (
            find(el, &format!("{t}Title")),
            find(el, &format!("{t}Sentence")),
        );
        let text = [
            title.map(inline).unwrap_or_default(),
            self.sentence(sentence),
        ]
        .into_iter()
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
        let children: Vec<El> = elements(el)
            .filter(|c| !same(title, *c) && !same(sentence, *c))
            .collect();
        let mut depth = depth;
        if !text.is_empty() || children.is_empty() {
            self.block(
                format!("{}- {}", "  ".repeat(depth), literal(&text))
                    .trim_end_matches(' ')
                    .to_string(),
            );
            depth += 1;
        }
        // else: markdown cannot nest under an empty item, so its children take its place
        for child in children {
            match tag(child) {
                t if is_item(t) => self.item(child, depth),
                "TableStruct" => self.table(child, &"  ".repeat(depth)),
                "List" => self.list(child, depth),
                _ => self.fallback(child, &"  ".repeat(depth)),
            }
        }
    }

    fn list(&mut self, el: El, depth: usize) {
        let mut nested = depth + 1;
        for child in elements(el) {
            let t = tag(child);
            if t.ends_with("Sentence") {
                let text = self.sentence(Some(child));
                if text.is_empty() && elements(el).any(|c| tag(c).starts_with("Sublist")) {
                    nested = depth; // as with an empty item, the sublists take its place
                    continue;
                }
                self.block(
                    format!("{}- {}", "  ".repeat(depth), literal(&text))
                        .trim_end_matches(' ')
                        .to_string(),
                );
            } else if t.starts_with("Sublist") {
                self.list(child, nested);
            } else {
                self.fallback(child, "");
            }
        }
    }

    fn figstruct(&mut self, el: El) {
        for child in elements(el) {
            if tag(child) == "Fig" {
                *self.unknown.entry("Fig".to_string()).or_default() += 1;
            } else {
                self.block(literal(&inline(child)));
            }
        }
    }

    fn table(&mut self, el: El, indent: &str) {
        for child in elements(el) {
            let cells: Vec<El> = if tag(child) == "Table" {
                elements(child).flat_map(|row| elements(row)).collect()
            } else {
                vec![child]
            };
            for text in cells.into_iter().map(inline) {
                if !text.is_empty() {
                    self.block(format!("{indent}{}", literal(&text)));
                }
            }
        }
    }

    fn suppl(&mut self, el: El) {
        let label = find(el, "SupplProvisionLabel");
        self.heading(2, label.map(inline).unwrap_or_default() + &suppl_source(el));
        for child in elements(el) {
            if same(label, child) {
                continue;
            }
            match tag(child) {
                "Article" => self.article(child, 3),
                "Paragraph" => self.paragraph(child),
                t if CONTAINERS.contains(&t) => self.container(child, 3),
                t if t.starts_with("SupplProvisionAppdx") => self.appendix(child, 3),
                _ => self.fallback(child, ""),
            }
        }
    }

    fn appendix(&mut self, el: El, level: usize) {
        let title_tag = format!("{}Title", tag(el));
        let children: Vec<El> = elements(el).collect();
        let n = children
            .iter()
            .take_while(|c| {
                [title_tag.as_str(), "RelatedArticleNum", "ArithFormulaNum"].contains(&tag(**c))
            })
            .count();
        self.heading(level, children[..n].iter().map(|c| inline(*c)).collect());
        for &child in &children[n..] {
            match tag(child) {
                "TableStruct" => self.table(child, ""),
                t if is_item(t) => self.item(child, 0),
                "Paragraph" => self.paragraph(child),
                _ => self.fallback(child, ""),
            }
        }
    }
}

/// `（平成一九年三月三一日法律第二三号）` for the 附則 of an amending law, so each block is
/// identifiable; the law's own 附則 has none. Written as e-Gov 法令検索 shows it.
pub fn suppl_source(el: El) -> String {
    el.attribute("AmendLawNum")
        .filter(|n| !n.is_empty())
        .map(|n| format!("（{n}）"))
        .unwrap_or_default()
}

pub fn parse(xml: &str) -> anyhow::Result<roxmltree::Document<'_>> {
    let options = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    Ok(roxmltree::Document::parse_with_options(xml, options)?)
}

pub fn law_body<'a, 'i>(doc: &'a roxmltree::Document<'i>) -> anyhow::Result<El<'a, 'i>> {
    find(doc.root_element(), "LawBody").ok_or_else(|| anyhow::anyhow!("no LawBody"))
}

pub fn render_doc(doc: &roxmltree::Document) -> anyhow::Result<Rendered> {
    let mut r = Renderer::default();
    r.body(law_body(doc)?);
    Ok(Rendered {
        markdown: r.blocks.join("\n\n") + "\n",
        unknown: r.unknown,
    })
}

pub fn render_law(xml: &str) -> anyhow::Result<Rendered> {
    render_doc(&parse(xml)?)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::fidelity::check;

    pub fn law(body: &str) -> String {
        format!("<Law><LawNum>テスト番号</LawNum><LawBody><LawTitle>テスト法</LawTitle>{body}</LawBody></Law>")
    }

    fn para(text: &str, num: &str) -> String {
        format!("<Paragraph><ParagraphNum>{num}</ParagraphNum><ParagraphSentence><Sentence>{text}</Sentence></ParagraphSentence></Paragraph>")
    }

    fn md(xml: &str) -> String {
        render_law(xml).unwrap().markdown
    }

    fn checked(xml: &str) -> Rendered {
        let r = render_law(xml).unwrap();
        check(xml, &r.markdown, "t").unwrap();
        r
    }

    #[test]
    fn chapter_article_paragraph_items() {
        let xml = law(&format!(
            "<MainProvision><Chapter><ChapterTitle>第一章　総則</ChapterTitle>\
             <Article><ArticleCaption>（目的）</ArticleCaption><ArticleTitle>第一条</ArticleTitle>{}\
             <Paragraph><ParagraphNum>２</ParagraphNum><ParagraphSentence><Sentence>前項の規定は、適用しない。</Sentence></ParagraphSentence>\
             <Item><ItemTitle>一</ItemTitle><ItemSentence><Sentence>国</Sentence></ItemSentence>\
             <Subitem1><Subitem1Title>イ</Subitem1Title><Subitem1Sentence><Sentence>内閣</Sentence></Subitem1Sentence></Subitem1>\
             </Item></Paragraph></Article></Chapter></MainProvision>",
            para("この法律は、目的とする。", "")
        ));
        assert_eq!(
            md(&xml),
            "# テスト法\n\n## 第一章　総則\n\n### 第一条（目的）\n\nこの法律は、目的とする。\n\n２ 前項の規定は、適用しない。\n\n- 一 国\n\n  - イ 内閣\n"
        );
    }

    #[test]
    fn suppl_provision_keeps_ideographic_space() {
        let xml = law(&format!(
            "<MainProvision>{}</MainProvision><SupplProvision><SupplProvisionLabel>附　則</SupplProvisionLabel>{}</SupplProvision>",
            para("本則。", ""),
            para("公布の日から施行する。", "")
        ));
        assert!(md(&xml).ends_with("## 附　則\n\n公布の日から施行する。\n"));
    }

    #[test]
    fn suppl_provision_of_an_amending_law_names_it() {
        let xml = law(&format!(
            "<MainProvision>{}</MainProvision><SupplProvision AmendLawNum=\"平成一九年三月三一日法律第二三号\" Extract=\"true\">\
             <SupplProvisionLabel>附　則</SupplProvisionLabel>{}</SupplProvision>",
            para("本則。", ""),
            para("公布の日から施行する。", "")
        ));
        let r = checked(&xml);
        assert!(r.markdown.ends_with(
            "## 附　則（平成一九年三月三一日法律第二三号）\n\n公布の日から施行する。\n"
        ));
    }

    #[test]
    fn heading_depth_clamps_at_six() {
        let xml = law(&format!(
            "<MainProvision><Part><PartTitle>第一編</PartTitle><Chapter><ChapterTitle>第一章</ChapterTitle>\
             <Section><SectionTitle>第一節</SectionTitle><Subsection><SubsectionTitle>第一款</SubsectionTitle>\
             <Division><DivisionTitle>第一目</DivisionTitle><Article><ArticleTitle>第一条</ArticleTitle>{}\
             </Article></Division></Subsection></Section></Chapter></Part></MainProvision>",
            para("本文。", "")
        ));
        let m = md(&xml);
        assert!(!m.contains("####### "));
        assert!(m.contains("###### 第一目") && m.contains("###### 第一条"));
    }

    #[test]
    fn table_cells_become_blocks() {
        let xml = law("<MainProvision><Paragraph><ParagraphNum/><ParagraphSentence><Sentence>次の表のとおり。</Sentence></ParagraphSentence>\
            <TableStruct><Table><TableRow><TableColumn><Sentence>元日</Sentence></TableColumn>\
            <TableColumn><Sentence>一月一日</Sentence></TableColumn></TableRow></Table></TableStruct></Paragraph></MainProvision>");
        assert!(md(&xml).ends_with("次の表のとおり。\n\n元日\n\n一月一日\n"));
    }

    #[test]
    fn toc_is_skipped_and_unknown_is_counted() {
        let xml = law(&format!(
            "<TOC><TOCLabel>目次</TOCLabel></TOC><MainProvision>{}<Mystery><Sentence>謎</Sentence></Mystery></MainProvision>",
            para("本文。", "")
        ));
        let r = render_law(&xml).unwrap();
        assert!(!r.markdown.contains("目次"));
        assert!(r.markdown.contains("\n\n謎\n"));
        assert_eq!(r.unknown["Mystery"], 1);
    }

    #[test]
    fn columns_are_space_separated() {
        let xml = law("<MainProvision><Paragraph><ParagraphNum/><ParagraphSentence><Column><Sentence>甲</Sentence></Column>\
            <Column><Sentence>乙</Sentence></Column></ParagraphSentence></Paragraph></MainProvision>");
        assert!(md(&xml).ends_with("甲 乙\n"));
    }

    #[test]
    fn appendix_table_heading_includes_related_article() {
        let xml = law(&format!(
            "<MainProvision>{}</MainProvision><AppdxTable><AppdxTableTitle>別表第一</AppdxTableTitle>\
             <RelatedArticleNum>（第二条関係）</RelatedArticleNum><TableStruct><Table><TableRow><TableColumn>\
             <Sentence>項目</Sentence></TableColumn></TableRow></Table></TableStruct></AppdxTable>",
            para("本文。", "")
        ));
        assert!(md(&xml).ends_with("## 別表第一（第二条関係）\n\n項目\n"));
    }

    #[test]
    fn appendix_arith_formula_num_keeps_order() {
        let xml = law(&format!(
            "<MainProvision>{}</MainProvision><Appdx><ArithFormulaNum>別記</ArithFormulaNum>\
             <RelatedArticleNum>（第二条関係）</RelatedArticleNum><ArithFormula><Sentence>式</Sentence></ArithFormula></Appdx>",
            para("本文。", "")
        ));
        assert!(md(&xml).ends_with("## 別記（第二条関係）\n\n式\n"));
    }

    #[test]
    fn empty_sentence_paragraph_has_no_trailing_space() {
        let xml = law("<MainProvision><Paragraph><ParagraphNum>２</ParagraphNum><ParagraphSentence/></Paragraph></MainProvision>");
        assert!(md(&xml).ends_with("\n\n２\n"));
    }

    #[test]
    fn empty_item_has_no_trailing_space() {
        let xml = law("<MainProvision><Paragraph><ParagraphNum/><ParagraphSentence><Sentence>本文。</Sentence></ParagraphSentence>\
            <Item><ItemTitle/><ItemSentence/><Subitem1><Subitem1Title/><Subitem1Sentence/></Subitem1></Item></Paragraph></MainProvision>");
        assert!(md(&xml).ends_with("本文。\n\n-\n"));
    }

    #[test]
    fn fixture_renders_have_no_trailing_ascii_whitespace_or_double_blank_lines() {
        for xml in crate::testutil::fixture_xmls() {
            let m = md(&xml);
            assert!(!m.split('\n').any(|l| l.ends_with(' ') || l.ends_with('\t')));
            assert!(!m.contains("\n\n\n"));
        }
    }

    #[test]
    fn list_under_paragraph_with_columns_and_sublist() {
        let xml = law("<MainProvision><Paragraph><ParagraphNum>１</ParagraphNum><ParagraphSentence><Sentence>次の通り。</Sentence></ParagraphSentence>\
            <List><ListSentence><Column><Sentence>第一</Sentence></Column><Column><Sentence>保管義務</Sentence></Column></ListSentence>\
            <Sublist1><Sublist1Sentence><Sentence>細目</Sentence></Sublist1Sentence>\
            <Sublist2><Sublist2Sentence><Column><Sentence>甲</Sentence></Column><Column><Sentence>乙</Sentence></Column></Sublist2Sentence></Sublist2>\
            </Sublist1></List></Paragraph></MainProvision>");
        let r = checked(&xml);
        assert!(r
            .markdown
            .ends_with("１ 次の通り。\n\n- 第一 保管義務\n\n  - 細目\n\n    - 甲 乙\n"));
        assert!(r.unknown.is_empty());
    }

    #[test]
    fn list_under_item() {
        let xml = law("<MainProvision><Paragraph><ParagraphNum/><ParagraphSentence><Sentence>本文。</Sentence></ParagraphSentence>\
            <Item><ItemTitle>一</ItemTitle><ItemSentence><Sentence>項目</Sentence></ItemSentence>\
            <List><ListSentence><Sentence>ア</Sentence></ListSentence></List></Item></Paragraph></MainProvision>");
        let r = checked(&xml);
        assert!(r.markdown.ends_with("- 一 項目\n\n  - ア\n"));
        assert!(r.unknown.is_empty());
    }

    #[test]
    fn enact_statement_is_one_block() {
        let xml = law(&format!("<EnactStatement>爆発物取締罰則別冊ノ通制定ス</EnactStatement><MainProvision>{}</MainProvision>", para("本文。", "")));
        let r = checked(&xml);
        assert!(r
            .markdown
            .starts_with("# テスト法\n\n爆発物取締罰則別冊ノ通制定ス\n\n"));
        assert!(r.unknown.is_empty());
    }

    #[test]
    fn preamble_renders_paragraphs() {
        let xml = law(&format!(
            "<Preamble>{}{}</Preamble><MainProvision>{}</MainProvision>",
            para("前文一。", "１"),
            para("前文二。", ""),
            para("本文。", "")
        ));
        let r = checked(&xml);
        assert!(r
            .markdown
            .starts_with("# テスト法\n\n１ 前文一。\n\n前文二。\n\n本文。\n"));
        assert!(r.unknown.is_empty());
    }

    #[test]
    fn figstruct_blocks_and_fig_counted() {
        let xml = law("<MainProvision><Paragraph><ParagraphNum/><ParagraphSentence><Sentence>本文。</Sentence></ParagraphSentence>\
            <FigStruct><FigStructTitle>図１</FigStructTitle><Remarks><RemarksLabel>備考</RemarksLabel><Sentence>注記</Sentence></Remarks>\
            <Fig src=\"./pict/x.jpg\"/></FigStruct></Paragraph></MainProvision>");
        let r = checked(&xml);
        assert!(r.markdown.ends_with("本文。\n\n図１\n\n備考注記\n"));
        assert_eq!(r.unknown, Counts::from([("Fig".to_string(), 1)]));
    }

    #[test]
    fn sup_sub_in_paragraph() {
        let xml = law(&format!(
            "<MainProvision>{}</MainProvision>",
            para("Ｘ<Sup>２</Sup>と<Sub>１</Sub>。", "")
        ));
        assert!(checked(&xml)
            .markdown
            .ends_with("Ｘ<sup>２</sup>と<sub>１</sub>。\n"));
    }

    #[test]
    fn table_and_unknown_inside_item_stay_in_the_list() {
        let table = "<TableStruct><TableStructTitle>表</TableStructTitle><Table><TableRow><TableColumn><Sentence>元日</Sentence></TableColumn>\
            <TableColumn><Sentence>一月一日</Sentence></TableColumn></TableRow></Table></TableStruct>";
        let xml = law(&format!(
            "<MainProvision><Paragraph><ParagraphNum/><ParagraphSentence><Sentence>本文。</Sentence></ParagraphSentence>\
             <Item><ItemTitle>一</ItemTitle><ItemSentence><Sentence>本</Sentence></ItemSentence>\
             <Subitem1><Subitem1Title>イ</Subitem1Title><Subitem1Sentence><Sentence>甲</Sentence></Subitem1Sentence>{table}\
             <Remarks><Sentence>備考</Sentence></Remarks></Subitem1>\
             <Subitem1><Subitem1Title>ロ</Subitem1Title><Subitem1Sentence><Sentence>乙</Sentence></Subitem1Sentence></Subitem1>\
             </Item></Paragraph></MainProvision>"
        ));
        assert!(checked(&xml).markdown.ends_with(
            "- 一 本\n\n  - イ 甲\n\n    表\n\n    元日\n\n    一月一日\n\n    備考\n\n  - ロ 乙\n"
        ));
    }

    #[test]
    fn children_of_an_empty_item_take_its_place() {
        let xml = "<Law><LawBody><MainProvision><Paragraph><ParagraphNum/><ParagraphSentence><Sentence>本文。</Sentence></ParagraphSentence>\
            <Item><ItemTitle>一</ItemTitle><ItemSentence><Sentence>号。</Sentence></ItemSentence>\
            <Subitem1><Subitem1Sentence/><Subitem2><Subitem2Sentence><Sentence>式</Sentence></Subitem2Sentence></Subitem2></Subitem1>\
            <Subitem1><Subitem1Sentence/></Subitem1></Item></Paragraph></MainProvision></LawBody></Law>";
        assert_eq!(md(xml), "本文。\n\n- 一 号。\n\n  - 式\n\n  -\n");
    }

    #[test]
    fn sublists_of_an_empty_list_sentence_take_its_place() {
        let xml = "<Law><LawBody><MainProvision><Paragraph><ParagraphNum/><ParagraphSentence><Sentence>本文。</Sentence></ParagraphSentence>\
            <List><ListSentence><Sentence/></ListSentence><Sublist1><Sublist1Sentence><Sentence/></Sublist1Sentence>\
            <Sublist2><Sublist2Sentence><Sentence>式</Sentence></Sublist2Sentence></Sublist2></Sublist1></List>\
            </Paragraph></MainProvision></LawBody></Law>";
        assert_eq!(md(xml), "本文。\n\n- 式\n");
    }
}
