import datetime
import threading
from concurrent.futures import ThreadPoolExecutor

from houritsu_builder.api import ApiError, Client
from houritsu_builder.cache import Cache

FUTURE = "2099-12-31"  # asof this date, a law's revision_info is its newest revision, enforced or not
_JST = datetime.timezone(datetime.timedelta(hours=9))


def fetch_law_list(client, **params) -> list[dict]:
    laws, offset = [], 0
    while True:
        page = client.get_json("laws", limit=1000, offset=offset, response_format="json", **params)
        laws += page["laws"]
        offset += len(page["laws"])
        if not page["laws"] or offset >= page["total_count"]:
            return laws


def _stale(cache: Cache, law_id: str, newest: list[dict | None], today: str) -> bool:
    path = cache.revisions_path(law_id)
    if not path.exists():
        return True
    revisions = cache.read(path)["revisions"]
    seen = {(r["law_revision_id"], r.get("updated")) for r in revisions}
    if any(info and (info["law_revision_id"], info.get("updated")) not in seen for info in newest):
        return True
    return any(r.get("current_revision_status") == "UnEnforced" and (r.get("amendment_enforcement_date") or "9999") <= today
               for r in revisions)


def _amendments(revisions: list[dict]) -> set[str]:
    return {r["amendment_law_id"] for r in revisions if r.get("amendment_law_id")}


def fetch(client, cache: Cache, law_type=None, law_ids=None, limit=None, log=print, jobs: int = 1, full: bool = False) -> dict:
    """Fetch revision lists and new revisions. Unless full, only for laws whose list may have changed."""
    if law_ids:
        targets, types = list(law_ids), {}
    else:
        laws = fetch_law_list(client)
        cache.write(cache.laws_path(), laws)
        types = {l["law_info"]["law_id"]: l["law_info"]["law_type"] for l in laws}
        targets = [law_id for law_id, t in types.items() if law_type is None or t == law_type]
        if not full:
            future = {l["law_info"]["law_id"]: l.get("revision_info") for l in fetch_law_list(client, asof=FUTURE)}
            current = {l["law_info"]["law_id"]: l.get("revision_info") for l in laws}
            today = datetime.datetime.now(_JST).date().isoformat()
            targets = [law_id for law_id in targets if _stale(cache, law_id, [current.get(law_id), future.get(law_id)], today)]
    targets.sort(key=lambda law_id: cache.revisions_path(law_id).exists())  # never-fetched laws first
    if limit:
        targets = targets[:limit]
    stats = {"laws": 0, "revisions": 0, "fetched": 0, "failed": 0}
    lock = threading.Lock()
    new_amendments: set[str] = set()

    def count(key: str, n: int = 1) -> None:
        with lock:
            stats[key] += n

    def fail(kind: str, key: str, error: Exception) -> None:
        with lock:
            cache.record_failure(kind, key, str(error))
            stats["failed"] += 1

    def one(item: tuple[int, str]) -> None:
        i, law_id = item
        try:
            revisions = client.get_json(f"law_revisions/{law_id}")
        except ApiError as e:
            fail("law_revisions", law_id, e)
            return
        path = cache.revisions_path(law_id)
        if path.exists():
            added = _amendments(revisions["revisions"]) - _amendments(cache.read(path)["revisions"])
            with lock:
                new_amendments.update(added)
        cache.write(path, revisions)
        count("revisions", len(revisions["revisions"]))
        for rev in revisions["revisions"]:
            rid = rev["law_revision_id"]
            path = cache.data_path(rid)
            if path.exists():
                continue
            try:
                data = client.get_json(f"law_data/{rid}", law_full_text_format="xml", response_format="json")
            except ApiError as e:
                fail("law_data", rid, e)
                continue
            cache.write(path, data)
            count("fetched")
        count("laws")
        log(f"[{i}] {law_id} revisions={len(revisions['revisions'])}")

    done, queried, step = set(), set(), 0
    while targets:
        with ThreadPoolExecutor(jobs) as pool:
            list(pool.map(one, enumerate(targets, step + 1)))
        step += len(targets)
        done.update(targets)
        if full or law_ids:
            break
        # A new amending law can add a revision in the middle of a law's history, which the newest-revision
        # check misses; e-Gov lists every law it amends.
        targets = []
        for amendment in sorted(new_amendments - queried):
            queried.add(amendment)
            for l in fetch_law_list(client, amendment_law_id=amendment, asof=FUTURE, omit_current_revision_info="true"):
                law_id = l["law_info"]["law_id"]
                if law_id not in done and law_id not in targets and (law_type is None or types.get(law_id) == law_type):
                    targets.append(law_id)
        new_amendments.clear()
    return stats


def run(args) -> int:
    stats = fetch(Client(delay=args.delay), Cache(args.cache), law_type=args.law_type, law_ids=args.law_ids,
                  limit=args.limit, jobs=args.jobs, full=args.full)
    print(stats)
    return 0
