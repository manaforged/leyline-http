from __future__ import annotations

import json
import os
import platform as plat
import re
import shutil
import subprocess
import sys
import tarfile
from pathlib import Path

from .captures import h3_path, store, tcp_path
from .config import CACHE, FAMILIES, FF_RELEASES_URL, FF_URL, H3_RUNS, H3_URL, HOST_OS, OUT, PEET_URL, ROOT
from .land import missing_majors
from .net import http_bytes, http_json
from .peet import require_browser_ua

def live_firefox_version() -> str:
    return http_json(FF_URL)["LATEST_FIREFOX_VERSION"]


def firefox_bin_for(ver: str) -> Path:
    if plat.system() == "Linux":
        if plat.machine().lower() not in {"x86_64", "amd64"}:
            raise SystemExit("Firefox collection on Linux requires x86_64")
        directory = CACHE / f"firefox-{ver}-linux-x86_64"
        binary = directory / "firefox" / "firefox"
        if binary.is_file() and os.access(binary, os.X_OK):
            return binary
        archive = CACHE / f"firefox-{ver}-linux-x86_64.tar.xz"
        url = (
            "https://download-installer.cdn.mozilla.net/pub/firefox/releases/"
            f"{ver}/linux-x86_64/en-US/firefox-{ver}.tar.xz"
        )
        print(f"downloading Firefox {ver}")
        http_bytes(url, archive)
        with tarfile.open(archive) as bundle:
            bundle.extractall(directory, filter="data")
        if not binary.is_file() or not os.access(binary, os.X_OK):
            raise SystemExit(f"Firefox executable missing under {directory}")
        return binary
    app = CACHE / f"Firefox-{ver}.app"
    binary = app / "Contents/MacOS/firefox"
    if binary.is_file() and os.access(binary, os.X_OK):
        return binary
    dmg = CACHE / f"Firefox-{ver}.dmg"
    url = (
        "https://download-installer.cdn.mozilla.net/pub/firefox/releases/"
        f"{ver}/mac/en-US/Firefox%20{ver}.dmg"
    )
    print(f"downloading Firefox {ver}")
    http_bytes(url, dmg)
    attached = subprocess.check_output(
        ["hdiutil", "attach", "-nobrowse", "-readonly", str(dmg)], text=True
    )
    mount = None
    for line in attached.splitlines():
        if "/Volumes/" in line:
            mount = line.split("/Volumes/", 1)[-1]
            mount = "/Volumes/" + mount.strip()
            break
    if not mount:
        raise SystemExit("hdiutil did not mount Firefox dmg")
    try:
        src = Path(mount) / "Firefox.app"
        if app.exists():
            shutil.rmtree(app)
        shutil.copytree(src, app)
    finally:
        subprocess.run(["hdiutil", "detach", mount], check=False, stdout=subprocess.DEVNULL)
    return binary


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
    if FAMILIES["firefox"]["h3"] and host == "linux":
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
                binary = firefox_bin_for(ver)
            except Exception as exc:
                last_err = exc
                continue
            captured.append(("firefox", major, capture_firefox(major, ver, binary)))
            break
        else:
            raise SystemExit(f"firefox {major}: could not download/dump ({last_err})")
    return captured
