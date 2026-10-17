import collections
import re
import xml.etree.ElementTree as ET
from dataclasses import dataclass, field

from houritsu_builder.xmltext import inline

CONTAINERS = ("Part", "Chapter", "Section", "Subsection", "Division")
ITEMS = ("Item",) + tuple(f"Subitem{i}" for i in range(1, 11))
SKIPPED_TAGS = {"TOC"}
_MARKUP_START = re.compile(r"[-+*#>=]|\d+[.)]")


def literal(text: str) -> str:
    """Escape statute text that markdown would read as a list, heading, quote or setext underline."""
    return "\\" + text if _MARKUP_START.match(text) else text


@dataclass
class Rendered:
    markdown: str
    unknown: collections.Counter = field(default_factory=collections.Counter)


class _Renderer:
    def __init__(self):
        self.blocks: list[str] = []
        self.unknown = collections.Counter()

    def block(self, text: str) -> None:
        if text:
            self.blocks.append(text)

    def heading(self, level: int, text: str) -> None:
        if text:
            self.block("#" * min(level, 6) + " " + text)

    def fallback(self, el: ET.Element, indent: str = "") -> None:
        self.unknown[el.tag] += 1
        self.block(indent + literal(inline(el)) if inline(el) else "")

    def body(self, body: ET.Element) -> None:
        for el in body:
            if el.tag == "LawTitle":
                self.heading(1, inline(el))
            elif el.tag in SKIPPED_TAGS:
                continue
            elif el.tag == "EnactStatement":
                self.block(literal(inline(el)))
            elif el.tag == "Preamble":
                for p in el:
                    if p.tag == "Paragraph":
                        self.paragraph(p)
                    else:
                        self.fallback(p)
            elif el.tag == "MainProvision":
                self.provision(el, 2)
            elif el.tag == "SupplProvision":
                self.suppl(el)
            elif el.tag.startswith("Appdx"):
                self.appendix(el, 2)
            else:
                self.fallback(el)

    def provision(self, el: ET.Element, level: int) -> None:
        for child in el:
            if child.tag in CONTAINERS:
                self.container(child, level)
            elif child.tag == "Article":
                self.article(child, level)
            elif child.tag == "Paragraph":
                self.paragraph(child)
            else:
                self.fallback(child)

    def container(self, el: ET.Element, level: int) -> None:
        title = el.find(f"{el.tag}Title")
        self.heading(level, inline(title) if title is not None else "")
        self.provision([c for c in el if c is not title], level + 1)

    def article(self, el: ET.Element, level: int) -> None:
        title, caption = el.find("ArticleTitle"), el.find("ArticleCaption")
        self.heading(level, (inline(title) if title is not None else "") + (inline(caption) if caption is not None else ""))
        for child in el:
            if child is title or child is caption:
                continue
            if child.tag == "Paragraph":
                self.paragraph(child)
            else:
                self.fallback(child)

    def sentence(self, el: ET.Element | None) -> str:
        if el is None:
            return ""
        if el.find("Column") is not None:
            return " ".join(inline(c) for c in el)
        return inline(el)

    def paragraph(self, el: ET.Element) -> None:
        caption, num, sentence = el.find("ParagraphCaption"), el.find("ParagraphNum"), el.find("ParagraphSentence")
        if caption is not None:
            self.block(literal(inline(caption)))
        n = inline(num) if num is not None else ""
        self.block(literal(" ".join(t for t in (n, self.sentence(sentence)) if t)))
        for child in el:
            if child in (caption, num, sentence):
                continue
            if child.tag in ITEMS:
                self.item(child, 0)
            elif child.tag == "TableStruct":
                self.table(child)
            elif child.tag == "List":
                self.list_(child, 0)
            elif child.tag == "FigStruct":
                self.figstruct(child)
            else:
                self.fallback(child)

    def item(self, el: ET.Element, depth: int) -> None:
        title, sentence = el.find(f"{el.tag}Title"), el.find(f"{el.tag}Sentence")
        text = " ".join(t for t in ((inline(title) if title is not None else ""), self.sentence(sentence)) if t)
        children = [c for c in el if c is not title and c is not sentence]
        if text or not children:
            self.block(("  " * depth + "- " + literal(text)).rstrip(" "))
            depth += 1
        # else: markdown cannot nest under an empty item, so its children take its place
        for child in children:
            if child.tag in ITEMS:
                self.item(child, depth)
            elif child.tag == "TableStruct":
                self.table(child, "  " * depth)
            elif child.tag == "List":
                self.list_(child, depth)
            else:
                self.fallback(child, "  " * depth)

    def list_(self, el: ET.Element, depth: int) -> None:
        for child in el:
            if child.tag.endswith("Sentence"):
                self.block(("  " * depth + "- " + literal(self.sentence(child))).rstrip(" "))
            elif child.tag.startswith("Sublist"):
                self.list_(child, depth + 1)
            else:
                self.fallback(child)

    def figstruct(self, el: ET.Element) -> None:
        for child in el:
            if child.tag == "Fig":
                self.unknown["Fig"] += 1
            else:
                self.block(literal(inline(child)))

    def table(self, el: ET.Element, indent: str = "") -> None:
        for child in el:
            cells = [col for row in child for col in row] if child.tag == "Table" else [child]
            for text in map(inline, cells):
                if text:
                    self.block(indent + literal(text))

    def suppl(self, el: ET.Element) -> None:
        label = el.find("SupplProvisionLabel")
        self.heading(2, inline(label) if label is not None else "")
        for child in el:
            if child is label:
                continue
            if child.tag == "Article":
                self.article(child, 3)
            elif child.tag == "Paragraph":
                self.paragraph(child)
            elif child.tag in CONTAINERS:
                self.container(child, 3)
            elif child.tag.startswith("SupplProvisionAppdx"):
                self.appendix(child, 3)
            else:
                self.fallback(child)

    def appendix(self, el: ET.Element, level: int) -> None:
        head_tags = {f"{el.tag}Title", "RelatedArticleNum", "ArithFormulaNum"}
        children = list(el)
        n = 0
        while n < len(children) and children[n].tag in head_tags:
            n += 1
        self.heading(level, "".join(inline(c) for c in children[:n]))
        for child in children[n:]:
            if child.tag == "TableStruct":
                self.table(child)
            elif child.tag in ITEMS:
                self.item(child, 0)
            elif child.tag == "Paragraph":
                self.paragraph(child)
            else:
                self.fallback(child)


def render_law(xml: bytes) -> Rendered:
    root = ET.fromstring(xml)
    r = _Renderer()
    r.body(root.find("LawBody"))
    return Rendered("\n\n".join(r.blocks) + "\n", r.unknown)
