#!/usr/bin/env python3
from __future__ import annotations

import base64
import json
import os
import re
import subprocess
import sys
import tomllib
import urllib.request
import xml.etree.ElementTree as ET
from pathlib import Path

from update_notice import notify

ROOT = Path(__file__).resolve().parent.parent
PROFILES = ROOT / "crates/leyline/profiles"
AGENT = {"User-Agent": "leyline-release-check"}


def fetch(url: str) -> bytes:
    headers = dict(AGENT)
    token = os.environ.get("GITHUB_TOKEN")
    if token and url.startswith("https://api.github.com/"):
        headers["Authorization"] = f"Bearer {token}"
    with urllib.request.urlopen(urllib.request.Request(url, headers=headers), timeout=30) as resp:
        return resp.read()


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


def chrome_release() -> str:
    url = (
        "https://versionhistory.googleapis.com/v1/chrome/platforms/win/channels/stable/"
        "versions/all/releases?filter=fraction%3E%3D1,endtime=none&order_by=version%20desc"
    )
    releases = [r for r in json.loads(fetch(url))["releases"] if r.get("fraction") == 1]
    return releases[0]["version"]


def chrome() -> int:
    return major(chrome_release())


def firefox() -> int:
    url = "https://product-details.mozilla.org/1.0/firefox_versions.json"
    return major(json.loads(fetch(url))["LATEST_FIREFOX_VERSION"])


def edge() -> int:
    products = json.loads(fetch("https://edgeupdates.microsoft.com/api/products"))
    stable = next(p for p in products if p["Product"] == "Stable")
    return max(major(r["ProductVersion"]) for r in stable["Releases"] if r["Platform"] == "Windows")


def brave() -> int:
    release = json.loads(fetch("https://api.github.com/repos/brave/brave-browser/releases/latest"))
    if release.get("prerelease") or not release["name"].startswith("Release"):
        raise ValueError(f"latest Brave release is not a stable release: {release['name']}")
    return major(re.search(r"Chromium\s+(\d+)", release["name"]).group(1))


def opera() -> int:
    index = fetch("https://get.geo.opera.com/pub/opera/desktop/").decode()
    return max(major(v) for v in re.findall(r'href="(\d+\.\d+\.\d+\.\d+)/"', index))


def apple() -> int:
    feed = ET.fromstring(fetch("https://developer.apple.com/news/releases/rss/releases.rss"))
    titles = [item.findtext("title") or "" for item in feed.iter("item")]
    stable = [t for t in titles if re.match(r"iOS \d+(\.\d+)* \(", t) and "beta" not in t.lower()]
    return max(major(t.split()[1]) for t in stable)


def boringssl_revision(version: str) -> str:
    deps = base64.b64decode(
        fetch(f"https://chromium.googlesource.com/chromium/src/+/refs/tags/{version}/DEPS?format=TEXT")
    ).decode()
    return re.search(r"'boringssl_revision':\s*'([0-9a-f]{40})'", deps).group(1)


def boringssl() -> int:
    version = chrome_release()
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
    except Exception as error:
        print(f"::error::boringssl: check failed: {error}")
        failed += 1
    for name, live, bundled in checks:
        try:
            latest, have = live(), bundled()
        except Exception as error:
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
