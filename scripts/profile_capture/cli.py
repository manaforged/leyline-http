"""Capture new browser builds and land their profiles.

    scripts/profile-oneshot.sh status
    scripts/profile-oneshot.sh firefox                 # capture on this host, then land
    scripts/profile-oneshot.sh firefox --capture-only  # store this host's captures only
    scripts/profile-oneshot.sh firefox --major 157 --land firefox-157.0
    scripts/profile-oneshot.sh chrome --major 155 --land  # newest stored build per OS
    scripts/profile-oneshot.sh plan                       # JSON list of missing majors

A capture stores scrubbed JSON in crates/leyline/profiles/captures/. Landing
starts from the previous profile, changes only what the captures show, adds
the guide rows, README range, changelog entry, and QPACK golden row, and
leaves every other profile alone. Exit status 2 means a person must review
the listed items before the change is merged.

Firefox captures TLS and HTTP/2 headless on every host, and HTTP/3 headful
under Xvfb on Linux. Chrome comes from LEYLINE_CHROME or Google's stable
package for the host: the apt repository on Linux, the signed disk image on
macOS, and the signed enterprise installer on Windows; it refuses Chrome for
Testing and chrome-headless-shell. Firefox comes from Mozilla's release
server, and the macOS and Windows builds must carry Mozilla's signature. Safari uses Safari.app through
safaridriver. Brave comes from LEYLINE_BRAVE or Brave's release: the apt repository
on Linux, and the signed GitHub release disk image and standalone installer on
macOS and Windows. `opera` runs the Linux build from get.geo.opera.com and adds
its row to brands.toml. Edge follows the Chrome profiles. Mobile Safari needs a
hand capture.
"""
from __future__ import annotations

import argparse
import json
import platform as plat
import subprocess

from .captures import stored_builds, version_key
from .catalog import catalog, fill_edge, plan, print_catalog
from .chrome import fill_chromium
from .config import CACHE, OUT, ROOT, TODAY
from .docs import document
from .firefox import fill_firefox
from .land import land, os_phrase
from .opera import fill_opera
from .safari import fill_safari

NEEDS_REVIEW = 2


def land_all(items: list[tuple[str, int, list[str]]], date: str, sync: bool) -> int:
    review = []
    for family, major, builds in items:
        builds = sorted(builds or stored_builds(family, major), key=version_key)
        if not builds:
            raise SystemExit(f"no stored {family} {major} capture to land")
        landed = land(family, major, builds, date)
        changed, notes = document(family, major, landed.previous, builds[-1], os_phrase(landed.oses), landed.qpack)
        print(f"landed {landed.path.relative_to(ROOT)} from {', '.join(builds)} ({os_phrase(landed.oses)})")
        for line in landed.notes:
            print(f"  {line}")
        for path in changed:
            print(f"  updated {path}")
        review += [f"{family} {major}: {line}" for line in landed.review + notes]
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
        choices=["all", "status", "plan", "chrome", "brave", "firefox", "safari", "edge", "opera"],
    )
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--major", type=int, action="append", help="major to recapture")
    parser.add_argument("--capture-only", action="store_true", help="store this host's captures and stop")
    parser.add_argument(
        "--land",
        nargs="*",
        metavar="CAPTURED",
        help="land --major from stored captures, e.g. firefox-157.0; with no build, the newest stored build per OS",
    )
    parser.add_argument("--date", default=TODAY, help="date for verified_at and verified_against")
    parser.add_argument("--sync", action="store_true", help="run cargo truesight sync after landing")
    args = parser.parse_args(argv)
    if args.land is not None and (args.target not in {"chrome", "brave", "firefox", "safari"} or len(args.major or []) != 1):
        parser.error("--land needs one family and one --major")
    if args.land is not None and args.dry_run:
        parser.error("--land writes the profile and docs; it has no dry run")
    if args.land is None and args.target == "safari" and plat.system() != "Darwin":
        parser.error("Safari collection requires macOS")
    return args


def capture(target: str, dry: bool, majors: list[int] | None) -> list[tuple[str, int, str]]:
    captured: list[tuple[str, int, str]] = []
    if target in {"all", "chrome"}:
        captured += fill_chromium("chrome", dry, majors)
    if target in {"all", "brave"}:
        captured += fill_chromium("brave", dry, majors)
    if target in {"all", "firefox"}:
        captured += fill_firefox(dry, majors)
    if target in {"all", "safari"} and plat.system() == "Darwin":
        captured += fill_safari(dry, majors)
    if target in {"all", "edge"}:
        captured += fill_edge(dry, {major for family, major, _ in captured if family == "chrome"})
    if target in {"all", "opera"} and plat.system() == "Linux":
        captured += fill_opera(dry)
    elif target == "opera":
        print("opera: the brand row is read from the Linux build; skipped on this host")
    return captured


def main(argv: list[str]) -> int:
    args = parse(argv)
    if args.land is not None:
        return land_all([(args.target, args.major[0], args.land)], args.date, args.sync)
    if args.target == "plan":
        print(json.dumps(plan()))
        return 0
    CACHE.mkdir(parents=True, exist_ok=True)
    OUT.mkdir(parents=True, exist_ok=True)
    rows = catalog()
    print_catalog(rows)
    if args.target == "status":
        return 1 if any(r[3] == "fill" for r in rows) else 0
    captured = capture(args.target, args.dry_run, args.major)
    if args.capture_only or args.dry_run:
        return 0
    return land_all([(family, major, [build]) for family, major, build in captured], args.date, args.sync)
