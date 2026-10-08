#!/usr/bin/env python3
from __future__ import annotations

import re
import subprocess
import sys
import tomllib
import xml.etree.ElementTree as ET

from profile_capture.binaries import major_of
from profile_capture.chrome import boringssl_revision, live_brave_major, live_chrome_major
from profile_capture.config import PROFILES, ROOT
from profile_capture.firefox import live_firefox_version
from profile_capture.net import http_json, http_text
from profile_capture.opera import live_opera_version
from update_notice import notify

EDGE_PRODUCTS = "https://edgeupdates.microsoft.com/api/products"
APPLE_RELEASES = "https://developer.apple.com/news/releases/rss/releases.rss"


def major(text: str) -> int:
    return int(re.search(r"\d+", text).group())


def profile_majors(family: str, prefix: str = "") -> list[int]:
    return sorted(
        int(path.stem[len(prefix):])
        for path in (PROFILES / family).glob(f"{prefix}*.toml")
        if path.stem[len(prefix):].isdigit()
    )


def brand_versions(brand: str) -> list[int]:
    rows = tomllib.loads((PROFILES / "brands.toml").read_text())
    return sorted(major(v[0]) for v in rows[brand].get("versions", {}).values())


def chrome() -> int:
    return live_chrome_major()[0]


def firefox() -> int:
    return major_of(live_firefox_version())


def edge() -> int:
    products = http_json(EDGE_PRODUCTS)
    stable = next(p for p in products if p["Product"] == "Stable")
    return max(major(r["ProductVersion"]) for r in stable["Releases"] if r["Platform"] == "Windows")


def brave() -> int:
    return live_brave_major()[0]


def opera() -> int:
    return major_of(live_opera_version())


def apple() -> int:
    feed = ET.fromstring(http_text(APPLE_RELEASES))
    titles = [item.findtext("title") or "" for item in feed.iter("item")]
    stable = [t for t in titles if re.match(r"iOS \d+(\.\d+)* \(", t) and "beta" not in t.lower()]
    return max(major(t.split()[1]) for t in stable)


def boringssl() -> int:
    version = live_chrome_major()[1]
    wanted = boringssl_revision(version)
    tree = subprocess.run(
        ["git", "-C", str(ROOT), "ls-tree", "HEAD", "crates/leyline-bssl-sys/deps/boringssl"],
        capture_output=True, text=True, check=True,
    )
    have = tree.stdout.split()[2]
    status = "matches" if have == wanted else "DIFFERS"
    print(f"boringssl: bundled {have[:12]}, Chrome {version} ships {wanted[:12]}: {status}")
    if have != wanted:
        notify(
            f"Update BoringSSL to {wanted[:12]} from Chrome {version}",
            f"Chrome {version} ships BoringSSL {wanted}. leyline-bssl-sys bundles {have}. "
            "Move crates/leyline-bssl-sys/deps/boringssl to the Chrome revision.",
        )


def main() -> int:
    checks = [
        ("chrome", chrome, lambda: profile_majors("chrome")),
        ("firefox", firefox, lambda: profile_majors("firefox")),
        ("brave (chromium)", brave, lambda: profile_majors("brave")),
        ("edge (chromium)", edge, lambda: profile_majors("chrome")),
        ("opera", opera, lambda: brand_versions("Opera")),
        ("safari", apple, lambda: profile_majors("safari")),
        ("safari ios", apple, lambda: profile_majors("safari", "ios")),
        ("cfnetwork ios", apple, lambda: profile_majors("cfnetwork", "ios")),
    ]
    failed = 0
    try:
        boringssl()
    except (Exception, SystemExit) as error:
        print(f"::error::boringssl: check failed: {error}")
        failed += 1
    for name, live, bundled in checks:
        try:
            latest, have = live(), bundled()
        except (Exception, SystemExit) as error:
            print(f"::error::{name}: check failed: {error}")
            failed += 1
            continue
        newest = max(have) if have else 0
        status = "current" if newest >= latest else "NEW RELEASE, no profile"
        print(f"{name}: stable {latest}, newest profile {newest}: {status}")
        if newest < latest:
            notify(
                f"Add a {name} {latest} profile",
                f"{name} {latest} is the current stable release. The newest Leyline profile is "
                f"{name} {newest}. Capture {name} {latest} and add its profile under crates/leyline/profiles/.",
            )
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
