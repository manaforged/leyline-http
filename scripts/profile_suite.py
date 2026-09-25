#!/usr/bin/env python3
"""One command: catalog live versions, capture gaps, land profiles.

    scripts/profile-oneshot.sh              # fill chrome / firefox / safari / edge
    scripts/profile-oneshot.sh status
    scripts/profile-oneshot.sh --dry-run
    scripts/profile-oneshot.sh chrome|firefox|safari|edge

Installed Google Chrome (LEYLINE_CHROME) driven over the DevTools pipe with
--headless=new, Firefox official dmg, and Safari.app through safaridriver.
Edge is the ChromiumBrand overlay on the current Chrome hello (TLS/H2 stay
Chrome). Refuses Chrome for Testing, chrome-headless-shell, WKWebView, and any build whose user
agent is headless. Writes capture = "browser". Does not invent JA4. No Brave.
Mobile Safari has no automated capture.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform as plat
import re
import shutil
import signal
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.request
from datetime import date
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PEET_URL = os.environ.get("PEET_URL", "https://tls.peet.ws/api/all")
FF_URL = "https://product-details.mozilla.org/1.0/firefox_versions.json"
FF_RELEASES_URL = "https://product-details.mozilla.org/1.0/firefox.json"
CACHE = Path(os.environ.get("LEYLINE_CFT_CACHE", Path.home() / ".cache/leyline-cft"))
OUT = Path(os.environ.get("LEYLINE_ONESHOT_OUT", Path(tempfile.gettempdir()) / "leyline-oneshot"))
TODAY = date.today().isoformat()
CHROME_DEB_BASE = "https://dl.google.com/linux/chrome/deb/"
CHROME_DEB_INDEX = CHROME_DEB_BASE + "dists/stable/main/binary-amd64/Packages"
MAC_CHROME = Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")


def http_json(url: str, timeout: int = 30) -> dict:
    with urllib.request.urlopen(url, timeout=timeout) as resp:
        return json.loads(resp.read())


def http_bytes(url: str, dest: Path, timeout: int = 300) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    with urllib.request.urlopen(url, timeout=timeout) as resp, dest.open("wb") as out:
        shutil.copyfileobj(resp, out)


def bundled_majors(family: str) -> list[int]:
    files = list((ROOT / "crates/leyline/profiles" / family).glob("*.toml"))
    majors: list[int] = []
    for path in files:
        try:
            majors.append(int(path.stem))
        except ValueError:
            continue
    return sorted(majors)


def skeleton_toml(family: str, major: int) -> Path:
    majors = bundled_majors(family)
    lower = [m for m in majors if m < major]
    if major in majors:
        src_major = major
    elif lower:
        src_major = lower[-1]
    else:
        raise SystemExit(f"no {family} skeleton for {major}")
    return ROOT / "crates/leyline/profiles" / family / f"{src_major}.toml"


def grab_toml(path: Path, key: str) -> str:
    m = re.search(rf'(?m)^{re.escape(key)}\s*=\s*"(.*)"\s*$', path.read_text())
    return m.group(1) if m else ""


def peet_get(obj: dict, *keys: str) -> str:
    cur: object = obj
    for key in keys:
        if not isinstance(cur, dict):
            return ""
        cur = cur.get(key)
    return cur if isinstance(cur, str) else ""


def extract_json_blob(raw: Path) -> dict:
    text = raw.read_text(errors="replace")
    m = re.search(r"(\{.*\})", text, re.S)
    if not m:
        raise SystemExit(f"no JSON object in {raw}")
    return json.loads(m.group(1))


def headers_from_peet(peet: dict) -> list[str]:
    frames = (peet.get("http2") or {}).get("sent_frames") or []
    for frame in frames:
        if frame.get("frame_type") == "HEADERS":
            return list(frame.get("headers") or [])
    return []


def chrome_grease(peet: dict, major: int) -> str:
    """GREASE token from the dump; Google Chrome in Chrome slot order. Not HeadlessChrome."""
    grease = None
    for line in headers_from_peet(peet):
        if not line.lower().startswith("sec-ch-ua:"):
            continue
        value = line.split(":", 1)[1].strip()
        for brand, ver in re.findall(r'"([^"]+)";v="(\d+)"', value):
            if brand in {"Chromium", "Google Chrome", "HeadlessChrome"}:
                continue
            grease = (brand, ver)
            break
    if grease is None:
        grease = ("Not?A_Brand", "24")
    brand, ver = grease
    return f'"{brand}";v="{ver}", "Chromium";v="{major}", "Google Chrome";v="{major}"'


def ciphers_from_peet(peet: dict) -> list[str]:
    raw = (peet.get("tls") or {}).get("ciphers") or []
    out = []
    for name in raw:
        if not isinstance(name, str):
            continue
        if "GREASE" in name.upper():
            continue
        out.append(name.split(" (", 1)[0].strip())
    return out


def require_browser_ua(peet: dict, product: str, source: str) -> str:
    ua = peet.get("user_agent") or ""
    if not ua:
        raise SystemExit(f"{source}: peet dump has no user agent; refusing to land a profile")
    if "HeadlessChrome" in ua or "Headless" in ua:
        raise SystemExit(f"{source}: user agent {ua!r} is a headless build; refusing to land it")
    if product not in ua:
        raise SystemExit(f"{source}: user agent {ua!r} lacks {product!r}; refusing to land it")
    return ua


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


def chrome_product_version(binary: Path) -> str:
    if "headless-shell" in str(binary).lower():
        raise SystemExit(f"{binary} is chrome-headless-shell; capture from the full Chrome build")
    out = subprocess.check_output([str(binary), "--version"], text=True).strip()
    m = re.fullmatch(r"Google Chrome (\d+\.\d+\.\d+\.\d+)", out)
    if not m:
        raise SystemExit(f"{binary} reports {out!r}, not Google Chrome; refusing to capture")
    return m.group(1)


class CdpPipe:
    def __init__(self, proc: subprocess.Popen, send_fd: int, recv_fd: int) -> None:
        self.proc = proc
        self.send_fd = send_fd
        self.recv_fd = recv_fd
        self.buf = b""
        self.next_id = 0

    def read_message(self, deadline: float) -> dict:
        while b"\0" not in self.buf:
            if time.monotonic() > deadline:
                raise SystemExit("Chrome DevTools pipe timed out")
            chunk = os.read(self.recv_fd, 65536)
            if not chunk:
                raise SystemExit("Chrome closed the DevTools pipe")
            self.buf += chunk
        raw, self.buf = self.buf.split(b"\0", 1)
        return json.loads(raw)

    def call(self, method: str, params: dict, deadline: float, session: str | None = None) -> dict:
        self.next_id += 1
        msg: dict = {"id": self.next_id, "method": method, "params": params}
        if session:
            msg["sessionId"] = session
        os.write(self.send_fd, json.dumps(msg).encode() + b"\0")
        while True:
            reply = self.read_message(deadline)
            if reply.get("id") != self.next_id:
                continue
            if "error" in reply:
                raise SystemExit(f"CDP {method} failed: {reply['error']}")
            return reply.get("result") or {}


def page_text_via_cdp(cmd: list[str], url: str, errp: Path, timeout: float = 45) -> str:
    chrome_in, send_fd = os.pipe()
    recv_fd, chrome_out = os.pipe()
    with errp.open("wb") as err:
        proc = subprocess.Popen(
            ["/bin/sh", "-c", f'exec "$@" 3<&{chrome_in} 4>&{chrome_out}', "sh",
             *cmd, "--remote-debugging-pipe", "about:blank"],
            stdin=subprocess.DEVNULL,
            stdout=err,
            stderr=err,
            pass_fds=(chrome_in, chrome_out),
            start_new_session=True,
        )
    os.close(chrome_in)
    os.close(chrome_out)
    cdp = CdpPipe(proc, send_fd, recv_fd)
    deadline = time.monotonic() + timeout
    try:
        target = cdp.call("Target.createTarget", {"url": url}, deadline)["targetId"]
        session = cdp.call("Target.attachToTarget", {"targetId": target, "flatten": True}, deadline)["sessionId"]
        while time.monotonic() < deadline:
            result = cdp.call(
                "Runtime.evaluate",
                {"expression": "document.readyState === 'complete' ? document.body.innerText : ''",
                 "returnByValue": True},
                deadline,
                session,
            )
            text = (result.get("result") or {}).get("value") or ""
            if text.strip().startswith("{"):
                return text
            time.sleep(0.5)
        raise SystemExit(f"{url} did not load within {timeout}s")
    finally:
        try:
            cdp.call("Browser.close", {}, time.monotonic() + 5)
        except (SystemExit, OSError):
            pass
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            os.killpg(proc.pid, signal.SIGKILL)
            proc.wait()
        os.close(send_fd)
        os.close(recv_fd)


def dump_chrome(chrome: Path, version: str, dump: Path) -> dict:
    OUT.mkdir(parents=True, exist_ok=True)
    udd = Path(tempfile.mkdtemp(prefix="chrome-user.", dir=OUT))
    raw = dump.with_suffix(".dump.txt")
    cmd = [
        str(chrome),
        "--headless=new",
        "--no-first-run",
        f"--user-agent={chrome_desktop_ua(version)}",
        f"--user-data-dir={udd}",
    ]
    if plat.system() == "Linux":
        cmd.append("--no-sandbox")
    try:
        text = page_text_via_cdp(cmd, PEET_URL, dump.with_suffix(".stderr"))
    finally:
        shutil.rmtree(udd, ignore_errors=True)
    raw.write_text(text)
    obj = extract_json_blob(raw)
    require_browser_ua(obj, f"Chrome/{version.split('.', 1)[0]}", f"Chrome {version}")
    dump.write_text(json.dumps(obj))
    return obj


def installed_chrome(major: int) -> tuple[str, Path] | None:
    path = os.environ.get("LEYLINE_CHROME")
    if not path:
        return None
    binary = Path(path)
    ver = chrome_product_version(binary)
    if int(ver.split(".", 1)[0]) != major:
        raise SystemExit(f"LEYLINE_CHROME is Chrome {ver}, not Chrome {major}")
    return ver, binary


def stable_chrome_deb() -> tuple[str, str, str]:
    with urllib.request.urlopen(CHROME_DEB_INDEX, timeout=30) as resp:
        index = resp.read().decode()
    for block in index.split("\n\n"):
        fields = dict(line.split(": ", 1) for line in block.splitlines() if ": " in line)
        if fields.get("Package") == "google-chrome-stable":
            return fields["Version"].split("-", 1)[0], fields["Filename"], fields["SHA256"]
    raise SystemExit("google-chrome-stable is missing from the Chrome apt index")


def linux_chrome(major: int) -> tuple[str, Path] | None:
    ver, filename, sha256 = stable_chrome_deb()
    if int(ver.split(".", 1)[0]) != major:
        return None
    directory = CACHE / f"chrome-{ver}-linux"
    binary = directory / "opt/google/chrome/chrome"
    if not binary.is_file():
        deb = CACHE / f"google-chrome-stable-{ver}.deb"
        print(f"downloading Google Chrome {ver}")
        http_bytes(CHROME_DEB_BASE + filename, deb)
        if hashlib.sha256(deb.read_bytes()).hexdigest() != sha256:
            deb.unlink()
            raise SystemExit(f"Google Chrome {ver}: package hash does not match the apt index")
        subprocess.run(["dpkg-deb", "-x", str(deb), str(directory)], check=True)
        deb.unlink()
    return chrome_product_version(binary), binary


def fetched_chrome(major: int) -> tuple[str, Path] | None:
    if plat.system() == "Linux":
        return linux_chrome(major)
    if plat.system() == "Darwin" and MAC_CHROME.is_file():
        ver = chrome_product_version(MAC_CHROME)
        if int(ver.split(".", 1)[0]) == major:
            return ver, MAC_CHROME
    return None


def chrome_for_major(major: int) -> tuple[str, Path]:
    installed = installed_chrome(major) or fetched_chrome(major)
    if installed is None:
        raise SystemExit(
            f"Chrome {major}: set LEYLINE_CHROME to an installed Google Chrome {major}; "
            "Chrome for Testing and chrome-headless-shell differ from the shipped browser"
        )
    return installed


def live_chrome_major() -> tuple[int, str]:
    data = http_json(
        "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions.json"
    )
    ver = data["channels"]["Stable"]["version"]
    return int(ver.split(".", 1)[0]), ver


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


def toml_str_list(items: list[str]) -> str:
    return "[\n" + "".join(f'    "{item}",\n' for item in items) + "]"


def iana_names() -> dict[str, dict[int, str]]:
    src = (ROOT / "crates/leyline/src/iana.rs").read_text()
    sigalgs = {
        int(num, 16): name
        for name, num in re.findall(r'\("(\w+)", 0x([0-9a-f]+)\)', src.split("struct Curve", 1)[0])
    }
    curves = {
        int(num, 16): name
        for name, num in re.findall(r'name: "(\w+)",\s*boring: "[^"]*",\s*id: 0x([0-9a-f]+)', src)
    }
    return {"supported_groups": curves, "signature_algorithms": sigalgs}


def peet_code(raw: str) -> int | None:
    m = re.fullmatch(r"0x([0-9a-fA-F]+)", raw) or re.search(r"\((?:0x([0-9a-fA-F]+)|(\d+))\)$", raw)
    if not m:
        return None
    hexval = m.group(1)
    return int(hexval, 16) if hexval else int(m.group(2))


def names_from_peet_ext(peet: dict, field: str) -> list[str]:
    table = iana_names().get(field, {})
    out: list[str] = []
    for ext in (peet.get("tls") or {}).get("extensions") or []:
        if not isinstance(ext, dict):
            continue
        raw = ext.get(field)
        if not isinstance(raw, list):
            continue
        for name in raw:
            if not isinstance(name, str) or "GREASE" in name.upper():
                continue
            code = peet_code(name)
            if code is not None and code & 0x0F0F == 0x0A0A:
                continue
            out.append(table.get(code) or name.split(" (", 1)[0].strip())
        if out:
            break
    return out


def clone_profile(src: Path, dest: Path, *, name: str, browser: str, version: int,
                  verified: str, captured: str, ja4: str, akamai: str,
                  ciphers: list[str] | None, identity: dict[str, tuple[str, str]],
                  curves: list[str] | None = None, sigalgs: list[str] | None = None) -> None:
    text = "\n".join(
        line for line in src.read_text().splitlines() if not line.lstrip().startswith("#")
    ) + "\n"
    text = re.sub(r'(?m)^name = ".*"$', f'name = "{name}"', text, count=1)
    text = re.sub(r'(?m)^browser = ".*"$', f'browser = "{browser}"', text, count=1)
    text = re.sub(r'(?m)^version = \d+$', f"version = {version}", text, count=1)
    text = re.sub(r'(?m)^variant = ".*"$', f'variant = "{name.replace(" ", "")}"', text, count=1)
    text = re.sub(r'(?m)^chromium_major = \d+$', f"chromium_major = {version}", text, count=1)
    text = re.sub(r'(?m)^hello = \d+\n', "", text, count=1)
    if re.search(r'(?m)^verified_against = ', text):
        text = re.sub(r'(?m)^verified_against = ".*"$', f'verified_against = "{verified}"', text, count=1)
    else:
        text = text.replace("[meta]\n", f"[meta]\nverified_against = \"{verified}\"\n", 1)
    if re.search(r'(?m)^verified_at = ', text):
        text = re.sub(r'(?m)^verified_at = ".*"$', f'verified_at = "{TODAY}"', text, count=1)
    else:
        text = text.replace("[meta]\n", f"[meta]\nverified_at = \"{TODAY}\"\n", 1)
    if re.search(r'(?m)^capture = ', text):
        text = re.sub(r'(?m)^capture = ".*"$', 'capture = "browser"', text, count=1)
    else:
        text = re.sub(r'(?m)^(version = \d+\n)', r'\1capture = "browser"\n', text, count=1)
    if re.search(r'(?m)^captured_against = ', text):
        text = re.sub(r'(?m)^captured_against = ".*"$', f'captured_against = "{captured}"', text, count=1)
    else:
        text = text.replace("[meta]\n", f"[meta]\ncaptured_against = \"{captured}\"\n", 1)
    text = re.sub(r'(?m)^ja4 = ".*"$', f'ja4 = "{ja4}"', text, count=1)
    text = re.sub(r'(?m)^akamai = ".*"$', f'akamai = "{akamai}"', text, count=1)
    if ciphers:
        text = re.sub(r"ciphers = \[[^\]]*\]", f"ciphers = {toml_str_list(ciphers)}", text, count=1, flags=re.S)
    if curves:
        text = re.sub(r"curves = \[[^\]]*\]", f"curves = {toml_str_list(curves)}", text, count=1, flags=re.S)
    if sigalgs:
        text = re.sub(r"sigalgs = \[[^\]]*\]", f"sigalgs = {toml_str_list(sigalgs)}", text, count=1, flags=re.S)
    for section, (ua, sch) in identity.items():
        pattern = rf'\[identity\.{section}\]\nuser_agent = "[^"]*"(\nsec_ch_ua = (?:\'[^\']*\'|""))?'

        def rewrite(m: re.Match, section: str = section, ua: str = ua, sch: str = sch) -> str:
            block = f'[identity.{section}]\nuser_agent = "{ua}"'
            if m.group(1) is not None:
                sec = f"'{sch}'" if sch else '""'
                block += f"\nsec_ch_ua = {sec}"
            return block

        updated, n = re.subn(pattern, rewrite, text, count=1)
        if n != 1:
            raise SystemExit(f"could not rewrite [identity.{section}] in {src}")
        text = updated
    dest.write_text(text)


def chrome_identity_blocks(major: int, sch: str) -> dict[str, tuple[str, str]]:
    return {
        "windows": (
            f"Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
            f"(KHTML, like Gecko) Chrome/{major}.0.0.0 Safari/537.36",
            sch,
        ),
        "macos": (
            f"Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 "
            f"(KHTML, like Gecko) Chrome/{major}.0.0.0 Safari/537.36",
            sch,
        ),
        "linux": (
            f"Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 "
            f"(KHTML, like Gecko) Chrome/{major}.0.0.0 Safari/537.36",
            sch,
        ),
        "android": (
            f"Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 "
            f"(KHTML, like Gecko) Chrome/{major}.0.0.0 Mobile Safari/537.36",
            sch,
        ),
    }


def safari_identity_blocks(major: int, ua: str) -> dict[str, tuple[str, str]]:
    if "Safari/" not in ua:
        ua = (
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) "
            f"AppleWebKit/605.1.15 (KHTML, like Gecko) Version/{major}.0 Safari/605.1.15"
        )
    return {"macos": (ua, "")}


def firefox_identity_blocks(major: int) -> dict[str, tuple[str, str]]:
    empty = ""
    return {
        "windows": (
            f"Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:{major}.0) Gecko/20100101 Firefox/{major}.0",
            empty,
        ),
        "macos": (
            f"Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:{major}.0) Gecko/20100101 Firefox/{major}.0",
            empty,
        ),
        "linux": (
            f"Mozilla/5.0 (X11; Linux x86_64; rv:{major}.0) Gecko/20100101 Firefox/{major}.0",
            empty,
        ),
        "android": (
            f"Mozilla/5.0 (Android 14; Mobile; rv:{major}.0) Gecko/{major}.0 Firefox/{major}.0",
            empty,
        ),
    }


def land_from_peet(family: str, major: int, peet: dict, captured: str, verified: str) -> Path:
    dest = ROOT / "crates/leyline/profiles" / family / f"{major}.toml"
    src = skeleton_toml(family, major)
    ja4 = peet_get(peet, "tls", "ja4")
    akamai = peet_get(peet, "http2", "akamai_fingerprint")
    if not ja4 or not akamai:
        raise SystemExit(f"{family} {major}: peet dump missing ja4/akamai")
    ciphers = ciphers_from_peet(peet)
    curves = names_from_peet_ext(peet, "supported_groups") or names_from_peet_ext(peet, "curves")
    sigalgs = names_from_peet_ext(peet, "signature_algorithms") or names_from_peet_ext(
        peet, "signature_algs"
    )
    if not ciphers:
        raise SystemExit(f"{family} {major}: peet dump had no ciphers")
    if not curves:
        raise SystemExit(f"{family} {major}: peet dump had no curves")
    if not sigalgs:
        raise SystemExit(f"{family} {major}: peet dump had no sigalgs")
    if family == "chrome":
        sch = chrome_grease(peet, major)
        identity = chrome_identity_blocks(major, sch)
        name, browser = f"Chrome {major}", "chrome"
    elif family == "firefox":
        identity = firefox_identity_blocks(major)
        name, browser = f"Firefox {major}", "firefox"
    elif family == "safari":
        identity = safari_identity_blocks(major, peet.get("user_agent") or "")
        name, browser = f"Safari {major}", "safari"
    else:
        raise SystemExit(f"land_from_peet does not handle {family}")
    clone_profile(
        src,
        dest,
        name=name,
        browser=browser,
        version=major,
        verified=verified,
        captured=captured,
        ja4=ja4,
        akamai=akamai,
        ciphers=ciphers,
        identity=identity,
        curves=curves or None,
        sigalgs=sigalgs or None,
    )
    print(f"wrote {dest.relative_to(ROOT)}  ja4={ja4}  akamai={akamai}")
    return dest


def ja4_groups(family: str) -> list[list[int]]:
    grouped: dict[str, list[int]] = {}
    for major in bundled_majors(family):
        path = ROOT / "crates/leyline/profiles" / family / f"{major}.toml"
        grouped.setdefault(grab_toml(path, "ja4"), []).append(major)
    return list(grouped.values())


def set_meta(text: str, key: str, value: str | None) -> str:
    line = rf"(?m)^{re.escape(key)} = .*\n"
    if value is None:
        return re.sub(line, "", text, count=1)
    if re.search(line, text):
        return re.sub(line, lambda _: f"{key} = {value}\n", text, count=1)
    updated, n = re.subn(r"(?m)^(version = \d+\n)", rf"\g<1>{key} = {value}\n", text, count=1)
    if n != 1:
        raise SystemExit(f"wire: no [meta] version line to anchor {key}")
    return updated


def wire_family(family: str, prefix: str) -> None:
    majors = bundled_majors(family)
    if not majors:
        return
    reps: dict[int, int] = {}
    for group in ja4_groups(family):
        for major in group:
            reps[major] = max(group)
    for major in majors:
        path = ROOT / "crates/leyline/profiles" / family / f"{major}.toml"
        text = path.read_text()
        text = set_meta(text, "variant", f'"{prefix}{major}"')
        if re.search(r"(?m)^chromium_major = ", text):
            text = set_meta(text, "chromium_major", str(major))
        rep = reps.get(major, major)
        text = set_meta(text, "hello", str(rep) if rep != major else None)
        path.write_text(text)

    print(f"wired {prefix}{majors[-1]}  meta written from toml majors + ja4 groups")


def missing_majors(family: str, live_major: int) -> list[int]:
    have = set(bundled_majors(family))
    if not have:
        return [live_major]
    return [m for m in range(min(have) + 1, live_major + 1) if m not in have]


def catalog() -> list[tuple[str, str, str, str, str]]:
    """family, bundled, live, action (fill/skip/ok), note"""
    rows: list[tuple[str, str, str, str, str]] = []
    chrome_have = bundled_majors("chrome")
    cmaj, cver = live_chrome_major()
    chrome_miss = missing_majors("chrome", cmaj)
    rows.append((
        "chrome",
        ",".join(str(m) for m in chrome_have) or "none",
        cver,
        "fill" if chrome_miss else "ok",
        f"missing {chrome_miss}" if chrome_miss else "current",
    ))
    ff_have = bundled_majors("firefox")
    ff_ver = live_firefox_version()
    ff_maj = int(ff_ver.split(".", 1)[0])
    firefox_miss = missing_majors("firefox", ff_maj)
    rows.append((
        "firefox",
        ",".join(str(m) for m in ff_have) or "none",
        ff_ver,
        "fill" if firefox_miss else "ok",
        f"missing {firefox_miss}" if firefox_miss else "current",
    ))
    if plat.system() == "Darwin":
        safari_host = safari_host_version()
        safari_maj = int(safari_host.split(".", 1)[0]) if safari_host[:1].isdigit() else 0
        safari_have = bundled_majors("safari")
        safari_gap = [] if safari_maj in safari_have else [safari_maj]
        rows.append((
            "safari",
            ",".join(str(m) for m in safari_have) or "none",
            safari_host,
            "fill" if safari_gap else "ok",
            f"Safari.app capture {safari_gap}" if safari_gap else "current",
        ))
    chrome_newest = chrome_have[-1] if chrome_have else 0
    edge_maj, edge_ver = live_edge_major()
    edge_gap = edge_maj > chrome_newest
    rows.append((
        "edge",
        f"overlay Chrome{chrome_newest}",
        edge_ver,
        "fill" if chrome_miss or edge_gap else "ok",
        "ChromiumBrand::Edge on current Chrome TLS" if not edge_gap
        else f"needs Chrome {edge_maj}",
    ))
    return rows


def safari_host_version() -> str:
    try:
        return subprocess.check_output(
            ["defaults", "read", "/Applications/Safari.app/Contents/Info", "CFBundleShortVersionString"],
            text=True,
        ).strip()
    except subprocess.CalledProcessError:
        return "?"


def live_edge_major() -> tuple[int, str]:
    try:
        ver = subprocess.check_output(
            ["defaults", "read", "/Applications/Microsoft Edge.app/Contents/Info",
             "CFBundleShortVersionString"],
            text=True,
            stderr=subprocess.DEVNULL,
        ).strip()
        return int(ver.split(".", 1)[0]), ver
    except (subprocess.CalledProcessError, ValueError, FileNotFoundError):
        cmaj, cver = live_chrome_major()
        return cmaj, f"Chromium {cver} lockstep"


def sudo_ok() -> bool:
    return subprocess.run(["sudo", "-n", "true"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0


def print_catalog(rows: list[tuple[str, str, str, str, str]]) -> None:
    print(f"{'family':<12} {'bundled':<28} {'live':<22} {'do':<10} note")
    print("-" * 110)
    for fam, bundled, live, action, note in rows:
        print(f"{fam:<12} {bundled:<28} {live:<22} {action:<10} {note}")
    print("suite: " + ", ".join(row[0] for row in rows))


def fill_chrome(dry: bool, no_wire: bool, majors: list[int] | None = None) -> int:
    live_maj, live_ver = live_chrome_major()
    missing = sorted(set(majors)) if majors else missing_majors("chrome", live_maj)
    if not missing:
        print("chrome: current")
        return 0
    print(f"chrome: fill {missing} (live {live_ver})")
    if dry:
        return 0
    OUT.mkdir(parents=True, exist_ok=True)
    for major in missing:
        ver, binary = chrome_for_major(major)
        dump = OUT / f"chrome-{major}.peet.json"
        print(f"capturing Chrome {ver} from {binary}")
        peet = dump_chrome(binary, ver, dump)
        land_from_peet(
            "chrome",
            major,
            peet,
            captured=f"chrome-{ver}",
            verified=f"tls.peet.ws {TODAY} Chrome {ver} --headless=new",
        )
    if not no_wire:
        wire_family("chrome", "Chrome")
    return 0


def fill_firefox(dry: bool, no_wire: bool, majors: list[int] | None = None) -> int:
    live = live_firefox_version()
    live_maj = int(live.split(".", 1)[0])
    missing = sorted(set(majors)) if majors else missing_majors("firefox", live_maj)
    if not missing:
        print("firefox: current")
        return 0
    print(f"firefox: fill {missing} (live {live})")
    if dry:
        return 0
    OUT.mkdir(parents=True, exist_ok=True)
    peet_py = ROOT / "scripts/firefox-peet.py"
    for major in missing:
        last_err = None
        landed = False
        for ver in firefox_try_versions(major, live):
            try:
                binary = firefox_bin_for(ver)
            except Exception as exc:
                last_err = exc
                continue
            dump = OUT / f"firefox-{ver}.peet.json"
            print(f"capturing Firefox {ver}")
            subprocess.check_call([sys.executable, str(peet_py), str(binary), PEET_URL, str(dump)])
            peet = json.loads(dump.read_text())
            require_browser_ua(peet, f"Firefox/{major}", f"Firefox {ver}")
            land_from_peet(
                "firefox",
                major,
                peet,
                captured=f"firefox-{ver}",
                verified=f"tls.peet.ws {TODAY} Firefox {ver}",
            )
            landed = True
            break
        if not landed:
            raise SystemExit(f"firefox {major}: could not download/dump ({last_err})")
    if not no_wire:
        wire_family("firefox", "Firefox")
    return 0


SAFARI_APP = Path("/Applications/Safari.app")
SAFARIDRIVER = Path("/usr/bin/safaridriver")
SAFARIDRIVER_PORT = int(os.environ.get("LEYLINE_SAFARIDRIVER_PORT", "4444"))


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
    with urllib.request.urlopen(req, timeout=60) as resp:
        return json.loads(resp.read()).get("value") or {}


def dump_safari(short: str, dump: Path) -> dict:
    if not SAFARIDRIVER.exists():
        raise SystemExit(f"{SAFARIDRIVER} missing; Safari capture needs safaridriver")
    driver = subprocess.Popen(
        [str(SAFARIDRIVER), "--port", str(SAFARIDRIVER_PORT)], start_new_session=True
    )
    try:
        for _ in range(50):
            try:
                webdriver("GET", "/status")
                break
            except OSError:
                time.sleep(0.2)
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
    m = re.search(r"(\{.*\})", str(text), re.S)
    if not m:
        raise SystemExit(f"Safari dump was not JSON: {str(text)[:200]!r}")
    peet = json.loads(m.group(1))
    require_browser_ua(peet, f"Version/{short.split('.', 1)[0]}", f"Safari {short}")
    dump.write_text(json.dumps(peet))
    return peet


def fill_safari(dry: bool, no_wire: bool, majors: list[int] | None = None) -> int:
    host = safari_host_version()
    if not host[:1].isdigit():
        print("safari: no Safari.app")
        return 1
    major = int(host.split(".", 1)[0])
    if major in bundled_majors("safari") and major not in (majors or []):
        print(f"safari: current ({host})")
        return 0
    print(f"safari: fill {major} (host {host})")
    print("safari-ios: no automated capture; Mobile Safari profiles need a hand capture")
    if dry:
        return 0
    short, bundle = safari_build()
    OUT.mkdir(parents=True, exist_ok=True)
    dump = OUT / f"safari-{major}.peet.json"
    print(f"safari: Safari.app {short} ({bundle}) via safaridriver {PEET_URL}")
    peet = dump_safari(short, dump)
    land_from_peet(
        "safari",
        major,
        peet,
        captured=f"safari-{short}-{bundle}",
        verified=f"tls.peet.ws {TODAY} Safari.app {short} ({bundle}) safaridriver",
    )
    if not no_wire:
        wire_family("safari", "Safari")
    return 0


def fill_edge(dry: bool, no_wire: bool) -> int:
    chrome_have = bundled_majors("chrome")
    newest = chrome_have[-1] if chrome_have else 0
    edge_maj, edge_ver = live_edge_major()
    if edge_maj > newest:
        print(f"edge: Chrome {newest} behind Edge/Chromium {edge_ver}; fill chrome first")
        return fill_chrome(dry, no_wire)
    print(f"edge: overlay on Chrome {newest} (TLS/H2 Chrome, HTTP Edg/{newest})")
    if dry or no_wire:
        return 0
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument(
        "target",
        nargs="?",
        default="all",
        choices=["all", "status", "chrome", "firefox", "safari", "edge"],
    )
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--no-wire", action="store_true", help="write TOML only")
    parser.add_argument("--major", type=int, action="append", help="major to recapture")
    args = parser.parse_args(argv)
    if args.target == "safari" and plat.system() != "Darwin":
        parser.error("Safari collection requires macOS")
    CACHE.mkdir(parents=True, exist_ok=True)
    OUT.mkdir(parents=True, exist_ok=True)

    rows = catalog()
    print_catalog(rows)
    if args.target == "status":
        return 1 if any(r[3] == "fill" for r in rows) else 0

    rc = 0
    if args.target in {"all", "chrome"}:
        rc |= fill_chrome(args.dry_run, args.no_wire, args.major)
    if args.target in {"all", "firefox"}:
        rc |= fill_firefox(args.dry_run, args.no_wire, args.major)
    if args.target in {"all", "safari"} and plat.system() == "Darwin":
        rc |= fill_safari(args.dry_run, args.no_wire, args.major)
    if args.target in {"all", "edge"}:
        rc |= fill_edge(args.dry_run, args.no_wire)
    if args.target == "all" and not args.dry_run:
        leftover = [r[0] for r in catalog() if r[3] == "fill"]
        if leftover:
            print("still behind:", ", ".join(leftover))
            return 1
    return rc


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
