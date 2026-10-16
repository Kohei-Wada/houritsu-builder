import base64
from pathlib import Path

from houritsu_builder.cache import Cache

FIXTURE_CACHE = Path(__file__).parent / "fixtures" / "cache"
LAW_ID = "323AC1000000178"


def fixture_xml(revision_suffix: str) -> bytes:
    cache = Cache(FIXTURE_CACHE)
    data = cache.read(cache.data_path(f"{LAW_ID}_{revision_suffix}"))
    return base64.b64decode(data["law_full_text"])
