import argparse
import sys
from pathlib import Path

DEFAULT_CACHE = Path.home() / ".cache" / "houritsu"


def parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="houritsu-builder")
    sub = p.add_subparsers(dest="command", required=True)

    f = sub.add_parser("fetch", help="crawl e-Gov law API v2 into the cache")
    f.add_argument("--cache", type=Path, default=DEFAULT_CACHE)
    f.add_argument("--law-type", help="only this e-Gov law_type, e.g. Act")
    f.add_argument("--law-id", action="append", dest="law_ids", help="only these law IDs (repeatable)")
    f.add_argument("--limit", type=int, help="stop after this many laws")
    f.add_argument("--delay", type=float, default=1.0, help="seconds between requests")

    b = sub.add_parser("build", help="build the houritsu repository from the cache")
    b.add_argument("--cache", type=Path, default=DEFAULT_CACHE)
    b.add_argument("--out", type=Path, required=True, help="directory for the new repository")
    b.add_argument("--law-type", help="only this e-Gov law_type, e.g. Act")
    b.add_argument("--no-lint", action="store_true", help="skip markdownlint")
    return p


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    if args.command == "fetch":
        from houritsu_builder.fetch import run as run_fetch

        return run_fetch(args)
    from houritsu_builder.build import run as run_build

    return run_build(args)


if __name__ == "__main__":
    sys.exit(main())
