from __future__ import annotations

import json
import os
import platform as plat
import re
import subprocess
import sys
from pathlib import Path

from .binaries import firefox_binary
from .captures import h3_path, store, tcp_path
from .config import FAMILIES, FF_RELEASES_URL, FF_URL, H3_RUNS, H3_URL, HOST_OS, OUT, PEET_URL, ROOT
from .land import missing_majors
from .net import http_json
from .peet import require_browser_ua

def live_firefox_version() -> str:
    return http_json(FF_URL)["LATEST_FIREFOX_VERSION"]


def firefox_try_versions(major: int, latest: str) -> list[str]:
    if latest.startswith(f"{major}."):
        return [latest]
    releases = http_json(FF_RELEASES_URL).get("releases") or {}
    found = [
        key.removeprefix("firefox-")
        for key in releases
        if re.fullmatch(rf"firefox-{major}\.\d+(?:\.\d+)?", key)
    ]
    return sorted(found, key=lambda v: [int(part) for part in v.split(".")], reverse=True)


def firefox_dump(binary: Path, url: str, dump: Path, headful: bool = False) -> dict:
    command = [sys.executable, str(ROOT / "scripts/firefox-peet.py"), str(binary), url, str(dump)]
    if headful:
        command.append("--headful")
        if plat.system() == "Linux" and not os.environ.get("DISPLAY"):
            command = ["xvfb-run", "-a", *command]
    subprocess.check_call(command)
    try:
        return json.loads(dump.read_text())
    finally:
        dump.unlink(missing_ok=True)


def capture_firefox(major: int, ver: str, binary: Path) -> str:
    captured = f"firefox-{ver}"
    host = HOST_OS[plat.system()]
    print(f"capturing Firefox {ver} on {host}")
    peet = firefox_dump(binary, PEET_URL, OUT / f"{captured}.peet.json")
    require_browser_ua(peet, f"Firefox/{major}", f"Firefox {ver}")
    store(tcp_path(captured, host), peet, h3=False)
    if host in FAMILIES["firefox"]["h3_hosts"]:
        for run in range(1, H3_RUNS + 1):
            print(f"capturing Firefox {ver} HTTP/3, run {run}")
            capture = firefox_dump(binary, H3_URL, OUT / f"{captured}-h3-run{run}.json", headful=True)
            require_browser_ua(capture, f"Firefox/{major}", f"Firefox {ver} HTTP/3")
            store(h3_path(captured, host, run), capture, h3=True)
    return captured


def fill_firefox(dry: bool, majors: list[int] | None = None) -> list[tuple[str, int, str]]:
    live = live_firefox_version()
    live_maj = int(live.split(".", 1)[0])
    missing = sorted(set(majors)) if majors else missing_majors("firefox", live_maj)
    if not missing:
        print("firefox: current")
        return []
    print(f"firefox: fill {missing} (live {live})")
    if dry:
        return []
    OUT.mkdir(parents=True, exist_ok=True)
    captured = []
    for major in missing:
        last_err = None
        for ver in firefox_try_versions(major, live):
            try:
                binary = firefox_binary(ver)
            except Exception as exc:
                last_err = exc
                continue
            captured.append(("firefox", major, capture_firefox(major, ver, binary)))
            break
        else:
            raise SystemExit(f"firefox {major}: could not download/dump ({last_err})")
    return captured
