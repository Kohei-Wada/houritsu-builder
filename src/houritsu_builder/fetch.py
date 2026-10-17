import threading
from concurrent.futures import ThreadPoolExecutor

from houritsu_builder.api import ApiError, Client
from houritsu_builder.cache import Cache


def fetch_law_list(client) -> list[dict]:
    laws, offset = [], 0
    while True:
        page = client.get_json("laws", limit=1000, offset=offset, response_format="json")
        laws += page["laws"]
        offset += len(page["laws"])
        if not page["laws"] or offset >= page["total_count"]:
            return laws


def fetch(client, cache: Cache, law_type=None, law_ids=None, limit=None, log=print, jobs: int = 1) -> dict:
    if law_ids:
        targets = list(law_ids)
    else:
        laws = fetch_law_list(client)
        cache.write(cache.laws_path(), laws)
        targets = [l["law_info"]["law_id"] for l in laws if law_type is None or l["law_info"]["law_type"] == law_type]
    targets.sort(key=lambda law_id: cache.revisions_path(law_id).exists())  # never-fetched laws first
    if limit:
        targets = targets[:limit]
    stats = {"laws": 0, "revisions": 0, "fetched": 0, "failed": 0}
    lock = threading.Lock()

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
        cache.write(cache.revisions_path(law_id), revisions)
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
        log(f"[{i}/{len(targets)}] {law_id} revisions={len(revisions['revisions'])}")

    with ThreadPoolExecutor(jobs) as pool:
        list(pool.map(one, enumerate(targets, 1)))
    return stats


def run(args) -> int:
    stats = fetch(Client(delay=args.delay), Cache(args.cache), law_type=args.law_type, law_ids=args.law_ids, limit=args.limit, jobs=args.jobs)
    print(stats)
    return 0
