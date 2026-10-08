from __future__ import annotations

import base64
import platform as plat
import re
import shutil
import tempfile
from pathlib import Path
from urllib.parse import urlparse

from .binaries import chromium_binary, chromium_release, major_of
from .captures import h3_path, store, tcp_path
from .cdp import page_text
from .config import BORINGSSL_REVISION, CHROME_STABLE_URL, CHROMIUM_DEPS_URL, CHROMIUM_BUILDS, CHROMIUM_H3_FLAGS, FAMILIES, H3_RUNS, H3_URL, HOST_OS, OUT, PEET_URL
from .land import bundled_majors, missing_majors
from .net import http_json, http_text
from .peet import extract_json_blob, require_browser_ua


def chrome_desktop_ua(version: str) -> str:
    major = version.split(".", 1)[0]
    system = plat.system()
    if system == "Darwin":
        os_part = "Macintosh; Intel Mac OS X 10_15_7"
    elif system == "Windows":
        os_part = "Windows NT 10.0; Win64; x64"
    else:
        os_part = "X11; Linux x86_64"
    return (
        f"Mozilla/5.0 ({os_part}) AppleWebKit/537.36 (KHTML, like Gecko) "
        f"Chrome/{major}.0.0.0 Safari/537.36"
    )


def dump_chrome(
    binary: Path, source: str, url: str, user_agent: str | None, ua_marker: str | None, extra: tuple[str, ...] = ()
) -> dict:
    OUT.mkdir(parents=True, exist_ok=True)
    profile = Path(tempfile.mkdtemp(prefix="chrome-user.", dir=OUT))
    cmd = [str(binary), "--headless=new", "--no-first-run", *extra]
    if user_agent:
        cmd.append(f"--user-agent={user_agent}")
    if plat.system() == "Linux":
        cmd.append("--no-sandbox")
    try:
        text = page_text(cmd, profile, url, OUT / f"{profile.name}.stderr")
    finally:
        shutil.rmtree(profile, ignore_errors=True)
    obj = extract_json_blob(text, f"{source} page")
    if ua_marker:
        require_browser_ua(obj, ua_marker, source)
    return obj


def live_chrome_major() -> tuple[int, str]:
    ver = http_json(CHROME_STABLE_URL)["channels"]["Stable"]["version"]
    return major_of(ver), ver


def boringssl_revision(version: str) -> str:
    deps = base64.b64decode(http_text(CHROMIUM_DEPS_URL.format(version=version))).decode()
    found = re.search(BORINGSSL_REVISION, deps)
    if not found:
        raise SystemExit(f"Chrome {version} DEPS names no boringssl_revision")
    return found.group(1)


def live_brave_major() -> tuple[int, str]:
    release = chromium_release("brave")
    return release["chromium"], release["name"]


LIVE = {"chrome": live_chrome_major, "brave": live_brave_major}


def live_gap(family: str, live: int) -> list[int]:
    return [] if live in bundled_majors(family) else [live]


def fill_chromium(family: str, dry: bool, majors: list[int] | None = None) -> list[tuple[str, int, str]]:
    live_maj, live_ver = LIVE[family]()
    gaps = missing_majors(family, live_maj) if FAMILIES[family]["fill_gaps"] else live_gap(family, live_maj)
    missing = sorted(set(majors)) if majors else gaps
    if not missing:
        print(f"{family}: current")
        return []
    print(f"{family}: fill {missing} (live {live_ver})")
    if dry:
        return []
    OUT.mkdir(parents=True, exist_ok=True)
    captured = []
    for major in missing:
        ver, binary = chromium_binary(family, major)
        source = f"{CHROMIUM_BUILDS[family]['product']} {ver}"
        print(f"capturing {source} from {binary}")
        marker = FAMILIES[family]["ua_marker"].format(major=major)
        host = HOST_OS[plat.system()]
        peet = dump_chrome(binary, source, PEET_URL, chrome_desktop_ua(ver), marker)
        store(tcp_path(f"{family}-{ver}", host), peet, h3=False)
        if host in FAMILIES[family]["h3_hosts"]:
            quic = tuple(flag.format(host=urlparse(H3_URL).hostname) for flag in CHROMIUM_H3_FLAGS)
            for run in range(1, H3_RUNS + 1):
                print(f"capturing {source} HTTP/3, run {run}")
                capture = dump_chrome(binary, source, H3_URL, chrome_desktop_ua(ver), marker, quic)
                store(h3_path(f"{family}-{ver}", host, run), capture, h3=True)
        captured.append((family, major, f"{family}-{ver}"))
    return captured
