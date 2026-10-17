import http.client
import json
import time
import urllib.error
import urllib.parse
import urllib.request
from typing import Any, Callable

BASE = "https://laws.e-gov.go.jp/api/2"
BACKOFF = 2.0


class ApiError(Exception):
    pass


def _urlopen(url: str) -> bytes:
    with urllib.request.urlopen(url, timeout=120) as r:
        return r.read()


class Client:
    def __init__(
        self,
        base: str = BASE,
        delay: float = 0.1,
        retries: int = 5,
        opener: Callable[[str], bytes] | None = None,
        sleep: Callable[[float], None] = time.sleep,
    ):
        self.base = base
        self.delay = delay
        self.retries = retries
        self.opener = opener or _urlopen
        self.sleep = sleep

    def get_json(self, path: str, **params) -> Any:
        url = f"{self.base}/{path}"
        if params:
            url += "?" + urllib.parse.urlencode(params)
        last: Exception | None = None
        for attempt in range(self.retries):
            try:
                body = self.opener(url)
                value = json.loads(body)
            except urllib.error.HTTPError as e:
                if e.code == 404:
                    raise ApiError(f"404 {url}") from e
                last = e
            except (OSError, ValueError, http.client.HTTPException) as e:
                last = e
            else:
                self.sleep(self.delay)
                return value
            if attempt < self.retries - 1:
                self.sleep(BACKOFF * 2 ** attempt)
        raise ApiError(f"gave up after {self.retries} attempts: {url}") from last
