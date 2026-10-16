import pytest

from conftest import fixture_xml
from houritsu_builder.fidelity import FidelityError, check, md_text, xml_text
from houritsu_builder.render import render_law

SUFFIXES = ["20160101_426AC0000000043", "20190501_429AC0000000063", "20200101_430AC1000000057"]


@pytest.mark.parametrize("suffix", SUFFIXES)
def test_real_fixture_passes(suffix):
    xml = fixture_xml(suffix)
    check(xml, render_law(xml).markdown, suffix)


def test_dropped_character_fails():
    xml = fixture_xml(SUFFIXES[-1])
    md = render_law(xml).markdown.replace("元日", "元", 1)
    with pytest.raises(FidelityError, match="mismatch at char"):
        check(xml, md, "mutated")


def test_ascii_markup_in_text_survives():
    xml = "<Law><LawBody><LawTitle>法</LawTitle><MainProvision><Paragraph><ParagraphNum/><ParagraphSentence><Sentence>&lt;b&gt;*注*_x_</Sentence></ParagraphSentence></Paragraph></MainProvision></LawBody></Law>".encode()
    md = render_law(xml).markdown
    assert "<b>*注*_x_" in md
    check(xml, md, "ascii")


def test_article_caption_order_and_frontmatter_ignored():
    xml = "<Law><LawBody><LawTitle>法</LawTitle><MainProvision><Article><ArticleCaption>（目的）</ArticleCaption><ArticleTitle>第一条</ArticleTitle><Paragraph><ParagraphNum/><ParagraphSentence><Sentence>本文。</Sentence></ParagraphSentence></Paragraph></Article></MainProvision></LawBody></Law>".encode()
    assert xml_text(xml) == "法第一条（目的）本文。"
    assert md_text("---\ntitle: \"x\"\n---\n\n" + render_law(xml).markdown) == "法第一条（目的）本文。"


def test_sup_sub_tags_are_ignored():
    xml = b"<Law><LawBody><MainProvision><Paragraph><ParagraphSentence><Sentence>A<Sup>2</Sup>B<Sub>1</Sub></Sentence></ParagraphSentence></Paragraph></MainProvision></LawBody></Law>"
    check(xml, "A<sup>2</sup>B<sub>1</sub>\n", "t")
    with pytest.raises(FidelityError):
        check(xml, "A<sup>3</sup>B<sub>1</sub>\n", "t")


def test_toc_quoted_in_a_sentence_is_text_but_top_level_toc_is_skipped():
    xml = ("<Law><LawBody><LawTitle>法</LawTitle><TOC><TOCLabel>目次</TOCLabel></TOC><MainProvision><Paragraph><ParagraphNum/>"
           "<ParagraphSentence><Sentence>「<QuoteStruct><TOC><TOCChapter><ChapterTitle>第一章　総則</ChapterTitle></TOCChapter></TOC>"
           "</QuoteStruct>」に改める。</Sentence></ParagraphSentence></Paragraph></MainProvision></LawBody></Law>").encode()
    assert xml_text(xml) == "法「第一章　総則」に改める。"
    check(xml, render_law(xml).markdown, "t")


def test_quoted_article_keeps_document_order():
    xml = ("<Law><LawBody><MainProvision><Paragraph><ParagraphNum/><ParagraphSentence><Sentence>次の一条を加える。"
           "<QuoteStruct><Article><ArticleCaption>（目的）</ArticleCaption><ArticleTitle>第二条</ArticleTitle></Article></QuoteStruct>"
           "</Sentence></ParagraphSentence></Paragraph></MainProvision></LawBody></Law>").encode()
    assert xml_text(xml) == "次の一条を加える。（目的）第二条"
    check(xml, render_law(xml).markdown, "t")


def test_empty_item_marker_is_not_text():
    xml = ("<Law><LawBody><MainProvision><Paragraph><ParagraphNum/><ParagraphSentence><Sentence>本文。</Sentence></ParagraphSentence>"
           "<Item><ItemTitle/><ItemSentence><Sentence/></ItemSentence></Item>"
           "<Item><ItemTitle>三</ItemTitle><ItemSentence><Sentence>号</Sentence></ItemSentence></Item>"
           "</Paragraph></MainProvision></LawBody></Law>").encode()
    check(xml, render_law(xml).markdown, "t")
