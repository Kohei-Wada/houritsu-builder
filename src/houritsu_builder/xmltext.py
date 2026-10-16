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
            reading = "".join(_inline(rt) for rt in child.findall("Rt"))
            base = (child.text or "") + "".join(("" if c.tag == "Rt" else _inline(c)) + (c.tail or "") for c in child)
            parts.append(f"<ruby>{base}<rt>{reading}</rt></ruby>")
        else:
            parts.append(_inline(child))
        parts.append(child.tail or "")
    return "".join(parts)


def inline(el: ET.Element) -> str:
    return strip_pretty(_inline(el))
