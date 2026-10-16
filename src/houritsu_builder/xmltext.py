import re
import xml.etree.ElementTree as ET

_PRETTY = re.compile(r"\n[ \t]*")
_ASCII_WS = re.compile(r"[ \t\r\n]+")


def strip_pretty(s: str) -> str:
    return _PRETTY.sub("", s).strip(" \t\r\n")


def strip_ascii_ws(s: str) -> str:
    return _ASCII_WS.sub("", s)


def _inline(el: ET.Element) -> str:
    parts = [el.text or ""]
    for child in el:
        if child.tag == "Ruby":
            body = child.text or ""
            for c in child:
                body += f"<rt>{_inline(c)}</rt>" if c.tag == "Rt" else _inline(c)
                body += c.tail or ""
            parts.append(f"<ruby>{body}</ruby>")
        elif child.tag in ("Sup", "Sub"):
            t = child.tag.lower()
            parts.append(f"<{t}>{_inline(child)}</{t}>")
        else:
            parts.append(_inline(child))
        parts.append(child.tail or "")
    return "".join(parts)


def inline(el: ET.Element) -> str:
    return strip_pretty(_inline(el))
