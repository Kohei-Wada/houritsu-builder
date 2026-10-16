import http.client
import urllib.error

import pytest

from houritsu_builder.api import ApiError, Client


def test_spaces_requests_and_builds_query():
    calls, sleeps = [], []
    client = Client(base="https://x/api/2", delay=1.0, opener=lambda u: calls.append(u) or b'{"ok": 1}', sleep=sleeps.append)
    assert client.get_json("laws", limit=1000, offset=0) == {"ok": 1}
    assert calls == ["https://x/api/2/laws?limit=1000&offset=0"]
    assert sleeps == [1.0]


def test_retries_with_backoff_then_succeeds():
    attempts, sleeps = [], []

    def opener(url):
        attempts.append(url)
        if len(attempts) < 3:
            raise urllib.error.URLError("reset")
        return b"[]"

    client = Client(base="https://x", delay=1.0, opener=opener, sleep=sleeps.append)
    assert client.get_json("laws") == []
    assert sleeps == [2.0, 4.0, 1.0]


def test_gives_up_after_retries():
    def opener(url):
        raise urllib.error.URLError("down")

    client = Client(base="https://x", retries=2, opener=opener, sleep=lambda s: None)
    with pytest.raises(ApiError, match="gave up after 2 attempts"):
        client.get_json("laws")


def test_404_is_not_retried():
    attempts = []

    def opener(url):
        attempts.append(url)
        raise urllib.error.HTTPError(url, 404, "Not Found", {}, None)

    client = Client(base="https://x", opener=opener, sleep=lambda s: None)
    with pytest.raises(ApiError, match="404"):
        client.get_json("law_data/x")
    assert len(attempts) == 1


def test_give_up_keeps_last_error_and_skips_final_backoff():
    last = urllib.error.URLError("down")

    def opener(url):
        raise last

    sleeps = []
    client = Client(base="https://x", delay=1.0, retries=2, opener=opener, sleep=sleeps.append)
    with pytest.raises(ApiError) as exc:
        client.get_json("laws")
    assert exc.value.__cause__ is last
    assert sleeps == [2.0]


def _flaky(first):
    calls = []

    def opener(url):
        calls.append(url)
        if len(calls) == 1:
            if isinstance(first, Exception):
                raise first
            return first
        return b'{"ok": 1}'

    return opener


@pytest.mark.parametrize("first", [
    http.client.IncompleteRead(b""),
    b"\xff\xfe",
    urllib.error.HTTPError("https://x/laws", 500, "err", {}, None),
])
def test_transient_failures_are_retried(first):
    sleeps = []
    client = Client(base="https://x", delay=1.0, opener=_flaky(first), sleep=sleeps.append)
    assert client.get_json("laws") == {"ok": 1}
    assert sleeps == [2.0, 1.0]
