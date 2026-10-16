from houritsu_builder.render import render_law


def law(body: str) -> bytes:
    return f"<Law><LawNum>テスト番号</LawNum><LawBody><LawTitle>テスト法</LawTitle>{body}</LawBody></Law>".encode()


def para(text, num=""):
    return f'<Paragraph><ParagraphNum>{num}</ParagraphNum><ParagraphSentence><Sentence>{text}</Sentence></ParagraphSentence></Paragraph>'


def test_chapter_article_paragraph_items():
    xml = law(
        "<MainProvision><Chapter><ChapterTitle>第一章　総則</ChapterTitle>"
        "<Article><ArticleCaption>（目的）</ArticleCaption><ArticleTitle>第一条</ArticleTitle>"
        + para("この法律は、目的とする。")
        + "<Paragraph><ParagraphNum>２</ParagraphNum><ParagraphSentence><Sentence>前項の規定は、適用しない。</Sentence></ParagraphSentence>"
        "<Item><ItemTitle>一</ItemTitle><ItemSentence><Sentence>国</Sentence></ItemSentence>"
        "<Subitem1><Subitem1Title>イ</Subitem1Title><Subitem1Sentence><Sentence>内閣</Sentence></Subitem1Sentence></Subitem1>"
        "</Item></Paragraph></Article></Chapter></MainProvision>"
    )
    assert render_law(xml).markdown == (
        "# テスト法\n\n"
        "## 第一章　総則\n\n"
        "### 第一条（目的）\n\n"
        "この法律は、目的とする。\n\n"
        "２ 前項の規定は、適用しない。\n\n"
        "- 一 国\n\n"
        "  - イ 内閣\n"
    )


def test_suppl_provision_keeps_ideographic_space():
    xml = law("<MainProvision>" + para("本則。") + "</MainProvision>"
              "<SupplProvision><SupplProvisionLabel>附　則</SupplProvisionLabel>" + para("公布の日から施行する。") + "</SupplProvision>")
    assert render_law(xml).markdown.endswith("## 附　則\n\n公布の日から施行する。\n")


def test_heading_depth_clamps_at_six():
    xml = law(
        "<MainProvision><Part><PartTitle>第一編</PartTitle><Chapter><ChapterTitle>第一章</ChapterTitle>"
        "<Section><SectionTitle>第一節</SectionTitle><Subsection><SubsectionTitle>第一款</SubsectionTitle>"
        "<Division><DivisionTitle>第一目</DivisionTitle><Article><ArticleTitle>第一条</ArticleTitle>" + para("本文。")
        + "</Article></Division></Subsection></Section></Chapter></Part></MainProvision>"
    )
    md = render_law(xml).markdown
    assert "####### " not in md
    assert "###### 第一目" in md and "###### 第一条" in md


def test_table_cells_become_blocks():
    xml = law("<MainProvision><Paragraph><ParagraphNum/><ParagraphSentence><Sentence>次の表のとおり。</Sentence></ParagraphSentence>"
              "<TableStruct><Table><TableRow><TableColumn><Sentence>元日</Sentence></TableColumn>"
              "<TableColumn><Sentence>一月一日</Sentence></TableColumn></TableRow></Table></TableStruct></Paragraph></MainProvision>")
    assert render_law(xml).markdown.endswith("次の表のとおり。\n\n元日\n\n一月一日\n")


def test_toc_is_skipped_and_unknown_is_counted():
    xml = law("<TOC><TOCLabel>目次</TOCLabel></TOC><MainProvision>" + para("本文。") + "<Mystery><Sentence>謎</Sentence></Mystery></MainProvision>")
    r = render_law(xml)
    assert "目次" not in r.markdown
    assert "\n\n謎\n" in r.markdown
    assert r.unknown["Mystery"] == 1


def test_columns_are_space_separated():
    xml = law("<MainProvision><Paragraph><ParagraphNum/><ParagraphSentence><Column><Sentence>甲</Sentence></Column>"
              "<Column><Sentence>乙</Sentence></Column></ParagraphSentence></Paragraph></MainProvision>")
    assert render_law(xml).markdown.endswith("甲 乙\n")


def test_appendix_table_heading_includes_related_article():
    xml = law("<MainProvision>" + para("本文。") + "</MainProvision><AppdxTable><AppdxTableTitle>別表第一</AppdxTableTitle>"
              "<RelatedArticleNum>（第二条関係）</RelatedArticleNum><TableStruct><Table><TableRow><TableColumn>"
              "<Sentence>項目</Sentence></TableColumn></TableRow></Table></TableStruct></AppdxTable>")
    assert render_law(xml).markdown.endswith("## 別表第一（第二条関係）\n\n項目\n")


def test_appendix_arith_formula_num_keeps_order():
    xml = law("<MainProvision>" + para("本文。") + "</MainProvision><Appdx><ArithFormulaNum>別記</ArithFormulaNum>"
              "<RelatedArticleNum>（第二条関係）</RelatedArticleNum><ArithFormula><Sentence>式</Sentence></ArithFormula></Appdx>")
    assert render_law(xml).markdown.endswith("## 別記（第二条関係）\n\n式\n")


def test_empty_sentence_paragraph_has_no_trailing_space():
    xml = law("<MainProvision><Paragraph><ParagraphNum>２</ParagraphNum><ParagraphSentence/></Paragraph></MainProvision>")
    assert render_law(xml).markdown.endswith("\n\n２\n")


def test_empty_item_has_no_trailing_space():
    xml = law("<MainProvision><Paragraph><ParagraphNum/><ParagraphSentence><Sentence>本文。</Sentence></ParagraphSentence>"
              "<Item><ItemTitle/><ItemSentence/><Subitem1><Subitem1Title/><Subitem1Sentence/></Subitem1></Item></Paragraph></MainProvision>")
    assert render_law(xml).markdown.endswith("本文。\n\n-\n\n  -\n")


def _fixture_xmls():
    import base64
    import gzip
    import json

    from conftest import FIXTURE_CACHE, TABLE_FIXTURE

    paths = sorted((FIXTURE_CACHE / "data").rglob("*.json.gz")) + [TABLE_FIXTURE]
    return [base64.b64decode(json.loads(gzip.decompress(p.read_bytes()))["law_full_text"]) for p in paths]


def test_fixture_renders_have_no_trailing_ascii_whitespace_or_double_blank_lines():
    for xml in _fixture_xmls():
        md = render_law(xml).markdown
        assert not [l for l in md.split("\n") if l.endswith((" ", "\t"))]
        assert "\n\n\n" not in md


def _checked(xml):
    from houritsu_builder.fidelity import check

    r = render_law(xml)
    check(xml, r.markdown, "t")
    return r


def test_list_under_paragraph_with_columns_and_sublist():
    xml = law("<MainProvision><Paragraph><ParagraphNum>１</ParagraphNum><ParagraphSentence><Sentence>次の通り。</Sentence></ParagraphSentence>"
              "<List><ListSentence><Column><Sentence>第一</Sentence></Column><Column><Sentence>保管義務</Sentence></Column></ListSentence>"
              "<Sublist1><Sublist1Sentence><Sentence>細目</Sentence></Sublist1Sentence>"
              "<Sublist2><Sublist2Sentence><Column><Sentence>甲</Sentence></Column><Column><Sentence>乙</Sentence></Column></Sublist2Sentence></Sublist2>"
              "</Sublist1></List></Paragraph></MainProvision>")
    r = _checked(xml)
    assert r.markdown.endswith("１ 次の通り。\n\n- 第一 保管義務\n\n  - 細目\n\n    - 甲 乙\n")
    assert not r.unknown


def test_list_under_item():
    xml = law("<MainProvision><Paragraph><ParagraphNum/><ParagraphSentence><Sentence>本文。</Sentence></ParagraphSentence>"
              "<Item><ItemTitle>一</ItemTitle><ItemSentence><Sentence>項目</Sentence></ItemSentence>"
              "<List><ListSentence><Sentence>ア</Sentence></ListSentence></List></Item></Paragraph></MainProvision>")
    r = _checked(xml)
    assert r.markdown.endswith("- 一 項目\n\n  - ア\n")
    assert not r.unknown


def test_enact_statement_is_one_block():
    xml = law("<EnactStatement>爆発物取締罰則別冊ノ通制定ス</EnactStatement><MainProvision>" + para("本文。") + "</MainProvision>")
    r = _checked(xml)
    assert r.markdown.startswith("# テスト法\n\n爆発物取締罰則別冊ノ通制定ス\n\n")
    assert not r.unknown


def test_preamble_renders_paragraphs():
    xml = law("<Preamble>" + para("前文一。", "１") + para("前文二。") + "</Preamble><MainProvision>" + para("本文。") + "</MainProvision>")
    r = _checked(xml)
    assert r.markdown.startswith("# テスト法\n\n１ 前文一。\n\n前文二。\n\n本文。\n")
    assert not r.unknown


def test_figstruct_blocks_and_fig_counted():
    xml = law("<MainProvision><Paragraph><ParagraphNum/><ParagraphSentence><Sentence>本文。</Sentence></ParagraphSentence>"
              '<FigStruct><FigStructTitle>図１</FigStructTitle><Remarks><RemarksLabel>備考</RemarksLabel><Sentence>注記</Sentence></Remarks>'
              '<Fig src="./pict/x.jpg"/></FigStruct></Paragraph></MainProvision>')
    r = _checked(xml)
    assert r.markdown.endswith("本文。\n\n図１\n\n備考注記\n")
    assert dict(r.unknown) == {"Fig": 1}


def test_sup_sub_in_paragraph():
    xml = law("<MainProvision>" + para("Ｘ<Sup>２</Sup>と<Sub>１</Sub>。") + "</MainProvision>")
    r = _checked(xml)
    assert r.markdown.endswith("Ｘ<sup>２</sup>と<sub>１</sub>。\n")
