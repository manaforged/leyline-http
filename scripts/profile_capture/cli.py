"""Capture new browser builds and land their profiles.

    scripts/profile-oneshot.sh status
    scripts/profile-oneshot.sh firefox                 # capture on this host, then land
    scripts/profile-oneshot.sh firefox --capture-only  # store this host's captures only
    scripts/profile-oneshot.sh firefox --major 157 --land firefox-157.0

A capture stores scrubbed JSON in crates/leyline/profiles/captures/. Landing
starts from the previous profile, changes only what the captures show, adds
the guide rows, README range, changelog entry, and QPACK golden row, and
leaves every other profile alone. Exit status 2 means a person must review
the listed items before the change is merged.

Firefox captures TLS and HTTP/2 headless on every host, and HTTP/3 headful
under Xvfb on Linux. Chrome comes from LEYLINE_CHROME, the installed Google
Chrome on macOS, or Google's apt repository on Linux; it refuses Chrome for
Testing and chrome-headless-shell. Safari uses Safari.app through
safaridriver. Brave, Opera, Edge, and Mobile Safari need a hand capture.
"""
from __future__ import annotations

import argparse
import platform as plat
import subprocess

from .catalog import catalog, fill_edge, print_catalog
from .chrome import fill_chrome
from .config import CACHE, OUT, ROOT, TODAY
from .docs import document
from .firefox import fill_firefox
from .land import land, os_phrase
from .safari import fill_safari

NEEDS_REVIEW = 2


def land_all(items: list[tuple[str, int, str]], date: str, sync: bool) -> int:
    review = []
    for family, major, captured in items:
        landed = land(family, major, captured, date)
        changed = document(family, major, landed.previous, captured, os_phrase(landed.oses), landed.qpack)
        print(f"landed {landed.path.relative_to(ROOT)} from {captured} ({os_phrase(landed.oses)})")
        for line in landed.notes:
            print(f"  {line}")
        for path in changed:
            print(f"  updated {path}")
        review += [f"{family} {major}: {line}" for line in landed.review]
    if items:
        review.append("docs/guide/profiles.md prose: describe what changed, if anything")
    if items and sync:
        if subprocess.run(["cargo", "truesight", "sync"], cwd=ROOT).returncode != 0:
            review.append("cargo truesight sync failed")
    elif items:
        review.append("run cargo truesight sync")
    for line in review:
        print(f"review: {line}")
    return NEEDS_REVIEW if review else 0


def parse(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument(
        "target",
        nargs="?",
        default="all",
        choices=["all", "status", "chrome", "firefox", "safari", "edge"],
    )
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--major", type=int, action="append", help="major to recapture")
    parser.add_argument("--capture-only", action="store_true", help="store this host's captures and stop")
    parser.add_argument("--land", metavar="CAPTURED", help="land --major from stored captures, e.g. firefox-157.0")
    parser.add_argument("--date", default=TODAY, help="date for verified_at and verified_against")
    parser.add_argument("--sync", action="store_true", help="run cargo truesight sync after landing")
    args = parser.parse_args(argv)
    if args.target == "safari" and plat.system() != "Darwin":
        parser.error("Safari collection requires macOS")
    if args.land and (args.target not in {"chrome", "firefox", "safari"} or len(args.major or []) != 1):
        parser.error("--land needs one family and one --major")
    return args


def capture(target: str, dry: bool, majors: list[int] | None) -> list[tuple[str, int, str]]:
    captured: list[tuple[str, int, str]] = []
    if target in {"all", "chrome"}:
        captured += fill_chrome(dry, majors)
    if target in {"all", "firefox"}:
        captured += fill_firefox(dry, majors)
    if target in {"all", "safari"} and plat.system() == "Darwin":
        captured += fill_safari(dry, majors)
    if target in {"all", "edge"}:
        captured += fill_edge(dry, {major for family, major, _ in captured if family == "chrome"})
    return captured


def main(argv: list[str]) -> int:
    args = parse(argv)
    if args.land:
        return land_all([(args.target, args.major[0], args.land)], args.date, args.sync)
    CACHE.mkdir(parents=True, exist_ok=True)
    OUT.mkdir(parents=True, exist_ok=True)
    rows = catalog()
    print_catalog(rows)
    if args.target == "status":
        return 1 if any(r[3] == "fill" for r in rows) else 0
    captured = capture(args.target, args.dry_run, args.major)
    if args.capture_only or args.dry_run:
        return 0
    return land_all(captured, args.date, args.sync)
