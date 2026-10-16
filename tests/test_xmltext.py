import xml.etree.ElementTree as ET

from houritsu_builder.xmltext import inline, strip_ascii_ws, strip_pretty


def test_strip_pretty_keeps_ideographic_space():
    assert strip_pretty("\n      附　則\n    ") == "附　則"


def test_strip_ascii_ws():
    assert strip_ascii_ws(" a\tb\nc　d ") == "abc　d"


def test_inline_joins_sentences_without_pretty_whitespace():
    el = ET.fromstring("<S>\n  <Sentence>一。</Sentence>\n  <Sentence>二。</Sentence>\n</S>")
    assert inline(el) == "一。二。"


def test_inline_ruby():
    el = ET.fromstring("<Sentence><Ruby>𠮟<Rt>しつ</Rt></Ruby>責する。</Sentence>")
    assert inline(el) == "<ruby>𠮟<rt>しつ</rt></ruby>責する。"


def test_inline_ruby_multiple_rt_keeps_order():
    el = ET.fromstring("<S><Ruby>漢<Rt>かん</Rt>字<Rt>じ</Rt></Ruby></S>")
    assert inline(el) == "<ruby>漢<rt>かん</rt>字<rt>じ</rt></ruby>"


def test_inline_sup_sub():
    el = ET.fromstring("<Sentence>Ａ<Sup>２</Sup>、Ｂ<Sub>１</Sub>及び</Sentence>")
    assert inline(el) == "Ａ<sup>２</sup>、Ｂ<sub>１</sub>及び"
