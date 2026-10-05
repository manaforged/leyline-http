from __future__ import annotations

import json
import os
import signal
import subprocess
import time
import urllib.request

from .captures import store, tcp_path
from .config import PEET_URL, SAFARIDRIVER, SAFARIDRIVER_PORT, SAFARI_APP, WAITS
from .land import bundled_majors
from .peet import extract_json_blob, require_browser_ua

def safari_host_version() -> str:
    try:
        return subprocess.check_output(
            ["defaults", "read", "/Applications/Safari.app/Contents/Info", "CFBundleShortVersionString"],
            text=True,
        ).strip()
    except subprocess.CalledProcessError:
        return "?"


def safari_build() -> tuple[str, str]:
    info = SAFARI_APP / "Contents/Info"
    ident = subprocess.check_output(["defaults", "read", str(info), "CFBundleIdentifier"], text=True).strip()
    if ident != "com.apple.Safari":
        raise SystemExit(f"{SAFARI_APP} is {ident!r}, not com.apple.Safari; refusing to capture")
    short = safari_host_version()
    bundle = subprocess.check_output(["defaults", "read", str(info), "CFBundleVersion"], text=True).strip()
    return short, bundle


def webdriver(method: str, path: str, body: dict | None = None) -> dict:
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(
        f"http://127.0.0.1:{SAFARIDRIVER_PORT}{path}",
        data=data,
        method=method,
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=WAITS["webdriver_request"]) as resp:
        return json.loads(resp.read()).get("value") or {}


def dump_safari(short: str) -> dict:
    if not SAFARIDRIVER.exists():
        raise SystemExit(f"{SAFARIDRIVER} missing; Safari capture needs safaridriver")
    driver = subprocess.Popen(
        [str(SAFARIDRIVER), "--port", str(SAFARIDRIVER_PORT)], start_new_session=True
    )
    try:
        ready_by = time.monotonic() + WAITS["driver_ready"]
        while True:
            try:
                webdriver("GET", "/status")
                break
            except OSError:
                if time.monotonic() > ready_by:
                    raise SystemExit("safaridriver did not start") from None
                time.sleep(WAITS["driver_poll"])
        session = webdriver("POST", "/session", {"capabilities": {"alwaysMatch": {"browserName": "safari"}}})
        sid = session["sessionId"]
        caps = session.get("capabilities") or {}
        if str(caps.get("browserName")).lower() != "safari" or caps.get("browserVersion") != short:
            raise SystemExit(f"safaridriver opened {caps.get('browserName')} {caps.get('browserVersion')}, "
                             f"not Safari {short}; refusing to capture")
        try:
            webdriver("POST", f"/session/{sid}/url", {"url": PEET_URL})
            text = webdriver("POST", f"/session/{sid}/execute/sync",
                             {"script": "return document.body.innerText", "args": []})
        finally:
            webdriver("DELETE", f"/session/{sid}")
    finally:
        os.killpg(driver.pid, signal.SIGTERM)
        driver.wait()
    peet = extract_json_blob(str(text), f"Safari {short} page")
    require_browser_ua(peet, f"Version/{short.split('.', 1)[0]}", f"Safari {short}")
    return peet


def fill_safari(dry: bool, majors: list[int] | None = None) -> list[tuple[str, int, str]]:
    host = safari_host_version()
    if not host[:1].isdigit():
        print("safari: no Safari.app")
        return []
    major = int(host.split(".", 1)[0])
    if majors and set(majors) != {major}:
        raise SystemExit(f"safari: this host has Safari {major}; it cannot capture {sorted(set(majors))}")
    if major in bundled_majors("safari") and not majors:
        print(f"safari: current ({host})")
        return []
    print(f"safari: fill {major} (host {host})")
    print("safari-ios: no automated capture; Mobile Safari profiles need a hand capture")
    if dry:
        return []
    short, bundle = safari_build()
    print(f"safari: Safari.app {short} ({bundle}) via safaridriver {PEET_URL}")
    captured = f"safari-{short}-{bundle}"
    store(tcp_path(captured, "macos"), dump_safari(short), h3=False)
    return [("safari", major, captured)]
