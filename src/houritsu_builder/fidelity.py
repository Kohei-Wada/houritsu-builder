import re
import xml.etree.ElementTree as ET

from houritsu_builder.render import CONTAINERS, SKIPPED_TAGS
from houritsu_builder.xmltext import strip_ascii_ws

_FRONTMATTER = re.compile(r"\A---\n.*?\n---\n", re.S)
_PREFIX = re.compile(r"^(#{1,6} | *- | *-$)")
_ARTICLE_PARENTS = {"MainProvision", "SupplProvision", *CONTAINERS}
_TAGS = re.compile(r"</?(ruby|rt|sup|sub)>")
_ESCAPE = re.compile(r"^( *)\\(?=[-+*#>=\d])")


class FidelityError(Exception):
    pass


def _walk(el: ET.Element, out: list[str], parent: str = "", quoted: bool = False) -> None:
    children = list(el)
    quoted = quoted or el.tag == "AmendProvision"  # rendered as plain text in document order
    if el.tag == "Article" and parent in _ARTICLE_PARENTS and not quoted:  # rendered as title then caption
        caption = el.find("ArticleCaption")
        if caption is not None:
            children.remove(caption)
            title_at = next((i for i, c in enumerate(children) if c.tag == "ArticleTitle"), -1)
            children.insert(title_at + 1, caption)
    out.append(el.text or "")
    for c in children:
        _walk(c, out, el.tag, quoted)
        out.append(c.tail or "")


def xml_text(xml: bytes) -> str:
    out: list[str] = []
    body = ET.fromstring(xml).find("LawBody")
    out.append(body.text or "")
    for c in body:
        if c.tag not in SKIPPED_TAGS:
            _walk(c, out)
        out.append(c.tail or "")
    return strip_ascii_ws("".join(out))


def md_text(md: str) -> str:
    body = _FRONTMATTER.sub("", md, count=1)
    return strip_ascii_ws("".join(_TAGS.sub("", _ESCAPE.sub(r"\1", _PREFIX.sub("", line))) for line in body.split("\n")))


def check(xml: bytes, md: str, label: str) -> None:
    a, b = xml_text(xml), md_text(md)
    if a == b:
        return
    i = next((k for k, (x, y) in enumerate(zip(a, b)) if x != y), min(len(a), len(b)))
    raise FidelityError(f"{label}: text mismatch at char {i}: xml={a[max(0, i - 20):i + 20]!r} md={b[max(0, i - 20):i + 20]!r}")
