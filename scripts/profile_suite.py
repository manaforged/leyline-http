#!/usr/bin/env python3
"""One command: catalog live versions, capture gaps, land profiles.

    scripts/profile-oneshot.sh              # fill chrome / firefox / safari / edge
    scripts/profile-oneshot.sh status
    scripts/profile-oneshot.sh --dry-run
    scripts/profile-oneshot.sh chrome|firefox|safari|edge

Chrome for Testing + Firefox official dmg + WKWebView Safari. Edge is the
ChromiumBrand overlay on the current Chrome hello (TLS/H2 stay Chrome).
Does not invent JA4. Does not land HeadlessChrome as the UA. No Brave.
"""
from __future__ import annotations

import argparse
import json
import os
import platform as plat
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import urllib.request
import zipfile
from datetime import date
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PEET_URL = os.environ.get("PEET_URL", "https://tls.peet.ws/api/all")
CFT_STABLE = (
    "https://googlechromelabs.github.io/chrome-for-testing/"
    "last-known-good-versions-with-downloads.json"
)
CFT_MILESTONES = (
    "https://googlechromelabs.github.io/chrome-for-testing/"
    "latest-versions-per-milestone-with-downloads.json"
)
FF_URL = "https://product-details.mozilla.org/1.0/firefox_versions.json"
CACHE = Path(os.environ.get("LEYLINE_CFT_CACHE", Path.home() / ".cache/leyline-cft"))
OUT = Path(os.environ.get("LEYLINE_ONESHOT_OUT", Path(tempfile.gettempdir()) / "leyline-oneshot"))
TODAY = date.today().isoformat()


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
    if lower:
        src_major = lower[-1]
    elif major in majors:
        src_major = major
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


def dump_chrome_headless(chrome: Path, dump: Path) -> dict:
    OUT.mkdir(parents=True, exist_ok=True)
    udd = Path(tempfile.mkdtemp(prefix="chrome-user.", dir=OUT))
    raw = dump.with_suffix(".dump.html")
    errp = dump.with_suffix(".stderr")
    cmd = [
        str(chrome),
        "--no-first-run",
        "--timeout=20000",
        f"--user-data-dir={udd}",
        "--dump-dom",
        PEET_URL,
    ]
    with raw.open("wb") as out, errp.open("wb") as err:
        proc = subprocess.Popen(cmd, stdout=out, stderr=err, start_new_session=True)
        try:
            proc.wait(timeout=35)
        except subprocess.TimeoutExpired:
            os.killpg(proc.pid, signal.SIGKILL)
            proc.wait()
    obj = extract_json_blob(raw)
    dump.write_text(json.dumps(obj))
    return obj


def cft_plat() -> str:
    system = plat.system()
    machine = plat.machine().lower()
    if system == "Darwin":
        return "mac-arm64" if machine in {"arm64", "aarch64"} else "mac-x64"
    if system == "Linux":
        if machine in {"arm64", "aarch64"}:
            raise SystemExit("Chrome for Testing has no linux-arm64 chrome-headless-shell")
        return "linux64"
    if system == "Windows":
        return "win64"
    raise SystemExit(f"no Chrome for Testing platform for {system}/{plat.machine()}")


def chrome_shell_for_major(major: int) -> tuple[str, Path]:
    """Return (full_version, chrome-headless-shell binary), downloading if needed."""
    CACHE.mkdir(parents=True, exist_ok=True)
    if major == live_chrome_major()[0]:
        data = http_json(CFT_STABLE)
        channel = data["channels"]["Stable"]
        ver = channel["version"]
        downloads = channel.get("downloads", {})
    else:
        data = http_json(CFT_MILESTONES)
        mile = data.get("milestones", {}).get(str(major))
        if not mile:
            raise SystemExit(f"no CFT milestone for Chrome {major}")
        ver = mile["version"]
        downloads = mile.get("downloads", {})
    url = None
    for item in downloads.get("chrome-headless-shell", []):
        if item.get("platform") == cft_plat():
            url = item["url"]
            break
    if not url:
        raise SystemExit(f"no chrome-headless-shell for Chrome {major} {cft_plat()}")
    dest_dir = CACHE / f"headless-{ver}-{cft_plat()}"
    binary = next(dest_dir.rglob("chrome-headless-shell"), None)
    if binary is None or not os.access(binary, os.X_OK):
        zpath = CACHE / f"headless-{ver}-{cft_plat()}.zip"
        print(f"downloading {url}")
        http_bytes(url, zpath)
        if dest_dir.exists():
            shutil.rmtree(dest_dir)
        dest_dir.mkdir(parents=True)
        with zipfile.ZipFile(zpath) as zf:
            zf.extractall(dest_dir)
        binary = next(dest_dir.rglob("chrome-headless-shell"), None)
    if binary is None or not os.access(binary, os.X_OK):
        raise SystemExit(f"chrome-headless-shell missing under {dest_dir}")
    return ver, binary


def live_chrome_major() -> tuple[int, str]:
    data = http_json(
        "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions.json"
    )
    ver = data["channels"]["Stable"]["version"]
    return int(ver.split(".", 1)[0]), ver


def live_firefox_version() -> str:
    return http_json(FF_URL)["LATEST_FIREFOX_VERSION"]


def firefox_bin_for(ver: str) -> Path:
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
    return [f"{major}.0.1", f"{major}.0"]


def toml_str_list(items: list[str]) -> str:
    return "[\n" + "".join(f'    "{item}",\n' for item in items) + "]"


def names_from_peet_ext(peet: dict, field: str) -> list[str]:
    out: list[str] = []
    for ext in (peet.get("tls") or {}).get("extensions") or []:
        if not isinstance(ext, dict):
            continue
        raw = ext.get(field)
        if not isinstance(raw, list):
            continue
        for name in raw:
            if isinstance(name, str) and "GREASE" not in name.upper():
                out.append(name.split(" (", 1)[0].strip())
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
    if re.search(r'(?m)^verified_against = ', text):
        text = re.sub(r'(?m)^verified_against = ".*"$', f'verified_against = "{verified}"', text, count=1)
    else:
        text = text.replace("[meta]\n", f"[meta]\nverified_against = \"{verified}\"\n", 1)
    if re.search(r'(?m)^verified_at = ', text):
        text = re.sub(r'(?m)^verified_at = ".*"$', f'verified_at = "{TODAY}"', text, count=1)
    else:
        text = text.replace("[meta]\n", f"[meta]\nverified_at = \"{TODAY}\"\n", 1)
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
        sec = f"'{sch}'" if sch else '""'
        block = f'[identity.{section}]\nuser_agent = "{ua}"\nsec_ch_ua = {sec}'
        updated, n = re.subn(
            rf'\[identity\.{section}\]\nuser_agent = "[^"]*"\nsec_ch_ua = (?:\'[^\']*\'|"")',
            block,
            text,
            count=1,
        )
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
            f"Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 "
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


def insert_once(text: str, needle: str, extra: str) -> str:
    if extra.strip() in text:
        return text
    if needle not in text:
        raise SystemExit(f"wire: missing marker {needle!r}")
    return text.replace(needle, needle + extra, 1)


def pin_after_latest(bind: str, latest_arm: str, dedicated: str) -> str:
    if dedicated.strip() in bind:
        return bind
    if latest_arm not in bind:
        raise SystemExit(f"wire: missing latest arm {latest_arm!r}")
    return bind.replace(latest_arm, latest_arm + dedicated, 1)


def wire_family(family: str, prefix: str) -> None:
    majors = bundled_majors(family)
    if not majors:
        return
    newest = majors[-1]
    browser_rs = ROOT / "crates/leyline/src/profile/browser.rs"
    registry = ROOT / "crates/leyline/src/profile/registry.rs"
    bindings = ROOT / "crates/leyline-ffi/src/profile.rs"
    identity = ROOT / "crates/leyline/src/core/session/identity.rs"
    brand = ROOT / "crates/leyline/src/profile/brand.rs"
    text = browser_rs.read_text()
    for major in majors:
        variant = f"{prefix}{major}"
        if f"{variant}," not in text and f"{variant} =>" not in text:
            prev = [m for m in majors if m < major]
            if not prev:
                raise SystemExit(f"cannot insert {variant} with no previous major")
            prev_v = f"{prefix}{prev[-1]}"
            text = insert_once(text, f"    {prev_v},\n", f"    /// {prefix} {major}.\n    {variant},\n")
            text = insert_once(
                text,
                f"    Browser::{prev_v},\n",
                f"    Browser::{variant},\n",
            )
            text = insert_once(
                text,
                f'            Self::{prev_v} => ("{family}", {prev[-1]}),\n',
                f'            Self::{variant} => ("{family}", {major}),\n',
            )
            text = insert_once(
                text,
                f'            Self::{prev_v} => write!(f, "{prefix} {prev[-1]}"),\n',
                f'            Self::{variant} => write!(f, "{prefix} {major}"),\n',
            )
            if family == "chrome":
                text = insert_once(
                    text,
                    f"            Self::{prev_v} => Some({prev[-1]}),\n",
                    f"            Self::{variant} => Some({major}),\n",
                )
    count = len(re.findall(r"^    Browser::\w+,$", text, re.M))
    text = re.sub(
        r"pub const PROFILE_COUNT: usize = \d+;",
        f"pub const PROFILE_COUNT: usize = {count};",
        text,
        count=1,
    )
    groups = ja4_groups(family)
    arms = []
    reps = []
    for group in sorted(groups, key=lambda g: max(g), reverse=True):
        names = " | ".join(f"Self::{prefix}{m}" for m in group)
        rep = max(group)
        arms.append(f"            {names} => Self::{prefix}{rep},")
        reps.append(f"Self::{prefix}{rep}")
    def swap_hello(match: re.Match[str]) -> str:
        body = match.group(1)
        kept = [
            line
            for line in body.splitlines()
            if f"Self::{prefix}" not in line or "other =>" in line
        ]
        out = []
        inserted = False
        for line in kept:
            if "other => other" in line:
                out.extend(arms)
                inserted = True
            out.append(line)
        if not inserted:
            out.extend(arms)
        return "    pub fn hello_rep(self) -> Self {\n        match self {\n" + "\n".join(out) + "\n        }\n    }"

    text, n = re.subn(
        r"    pub fn hello_rep\(self\) -> Self \{\n        match self \{\n(.*?)\n        \}\n    \}",
        swap_hello,
        text,
        count=1,
        flags=re.S,
    )
    if n != 1:
        raise SystemExit("wire: could not rewrite hello_rep")
    hellos = ", ".join(reps)
    text = re.sub(
        rf'            "{family}" => &\[.*?\],',
        f'            "{family}" => &[{hellos}],',
        text,
        count=1,
    )
    if family == "chrome":
        text = re.sub(
            r"(pub fn default_browser\(\) -> Self \{\n        Self::)Chrome\d+",
            rf"\1Chrome{newest}",
            text,
            count=1,
        )
    if family == "firefox":
        text = re.sub(
            r"(pub fn default_firefox\(\) -> Self \{\n        Self::)Firefox\d+",
            rf"\1Firefox{newest}",
            text,
            count=1,
        )
    if family == "safari":
        text = re.sub(
            r"\(Self::Safari\d+, Platform::IOS\) => Self::SafariIOS18,",
            f"(Self::Safari18 | Self::Safari{newest}, Platform::IOS) => Self::SafariIOS18,"
            if newest != 18
            else "(Self::Safari18, Platform::IOS) => Self::SafariIOS18,",
            text,
            count=1,
        )
        text = re.sub(
            r"\(Self::SafariIOS17 \| Self::SafariIOS18, Platform::MacOS\) => Self::Safari\d+,",
            f"(Self::SafariIOS17 | Self::SafariIOS18, Platform::MacOS) => Self::Safari{newest},",
            text,
            count=1,
        )
    browser_rs.write_text(text)

    reg = registry.read_text()
    load_line = f'        reg.load_toml(include_str!("../../profiles/{family}/{newest}.toml"));\n'
    if load_line not in reg:
        prev = majors[-2] if len(majors) > 1 else None
        if prev is None:
            raise SystemExit("wire: registry has no previous include")
        prev_line = f'        reg.load_toml(include_str!("../../profiles/{family}/{prev}.toml"));\n'
        reg = insert_once(reg, prev_line, load_line)
        registry.write_text(reg)

    bind = bindings.read_text()
    if family == "chrome":
        bind = re.sub(
            r'"chrome" \| "chrome-latest" \| "chrome\d+" \| "chrome-\d+" => \{\n            \(Chrome\d+, Windows, Brand::Chrome\)',
            f'"chrome" | "chrome-latest" | "chrome{newest}" | "chrome-{newest}" => {{\n            (Chrome{newest}, Windows, Brand::Chrome)',
            bind,
            count=1,
        )
        if len(majors) > 1:
            older = majors[-2]
            latest_arm = (
                f'        "chrome" | "chrome-latest" | "chrome{newest}" | "chrome-{newest}" => {{\n'
                f'            (Chrome{newest}, Windows, Brand::Chrome)\n'
                f'        }}\n'
            )
            dedicated = (
                f'        "chrome{older}" | "chrome-{older}" => '
                f'(Chrome{older}, Windows, Brand::Chrome),\n'
            )
            bind = pin_after_latest(bind, latest_arm, dedicated)
        bind = re.sub(
            r'"edge" \| "edge-latest" \| "edge\d+" \| "edge-\d+" => \(Chrome\d+, Windows, Brand::Edge\)',
            f'"edge" | "edge-latest" | "edge{newest}" | "edge-{newest}" => (Chrome{newest}, Windows, Brand::Edge)',
            bind,
            count=1,
        )
        bind = re.sub(
            r'"opera" \| "opera-latest" => \(Chrome\d+, Windows, Brand::Opera\)',
            f'"opera" | "opera-latest" => (Chrome{newest}, Windows, Brand::Opera)',
            bind,
            count=1,
        )
    if family == "firefox":
        bind = re.sub(
            r'"firefox" \| "firefox-latest" \| "firefox\d+" \| "firefox-\d+" => \{\n            \(Firefox\d+, Windows, Brand::Chrome\)',
            f'"firefox" | "firefox-latest" | "firefox{newest}" | "firefox-{newest}" => {{\n            (Firefox{newest}, Windows, Brand::Chrome)',
            bind,
            count=1,
        )
        if len(majors) > 1:
            older = majors[-2]
            latest_arm = (
                f'        "firefox" | "firefox-latest" | "firefox{newest}" | "firefox-{newest}" => {{\n'
                f'            (Firefox{newest}, Windows, Brand::Chrome)\n'
                f'        }}\n'
            )
            dedicated = (
                f'        "firefox{older}" | "firefox-{older}" => '
                f'(Firefox{older}, Windows, Brand::Chrome),\n'
            )
            bind = pin_after_latest(bind, latest_arm, dedicated)
    if family == "safari":
        bind = re.sub(
            r'"safari" \| "safari-latest" \| "safari\d+" \| "safari-\d+" => \(Safari\d+, MacOS, Brand::Chrome\)',
            f'"safari" | "safari-latest" | "safari{newest}" | "safari-{newest}" => (Safari{newest}, MacOS, Brand::Chrome)',
            bind,
            count=1,
        )
        if len(majors) > 1:
            older = majors[-2]
            latest_arm = (
                f'        "safari" | "safari-latest" | "safari{newest}" | '
                f'"safari-{newest}" => (Safari{newest}, MacOS, Brand::Chrome),\n'
            )
            dedicated = (
                f'        "safari{older}" | "safari-{older}" => '
                f'(Safari{older}, MacOS, Brand::Chrome),\n'
            )
            bind = pin_after_latest(bind, latest_arm, dedicated)
    bindings.write_text(bind)

    ident = identity.read_text()
    if family == "chrome":
        ident = re.sub(
            r"        Browser::Chrome\d+,",
            f"        Browser::Chrome{newest},",
            ident,
            count=1,
        )
    if family == "firefox":
        ident = re.sub(
            r"        Browser::Firefox\d+,",
            f"        Browser::Firefox{newest},",
            ident,
            count=1,
        )
    identity.write_text(ident)

    if family == "chrome":
        btxt = brand.read_text()
        opera = newest - 16
        row = f"    ({newest}, {opera}),\n"
        if f"({newest}," not in btxt:
            btxt = insert_once(btxt, "const OPERA_PER_CHROMIUM: &[(u32, u32)] = &[\n", row)
            brand.write_text(btxt)
    if family == "safari":
        builder = ROOT / "crates/leyline/src/core/session/builder.rs"
        bld = builder.read_text()
        bld = bld.replace(
            "Browser::Safari18.for_platform",
            f"Browser::Safari{newest}.for_platform",
        )
        bld = bld.replace(
            "self.browser(Browser::Safari18).macos()",
            f"self.browser(Browser::Safari{newest}).macos()",
        )
        builder.write_text(bld)
    print(f"wired {prefix}{newest}  PROFILE_COUNT from toml majors + siblings")


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
    safari_host = safari_host_version()
    safari_maj = int(safari_host.split(".", 1)[0]) if safari_host[:1].isdigit() else 0
    safari_have = bundled_majors("safari")
    safari_gap = [] if safari_maj in safari_have else [safari_maj]
    rows.append((
        "safari",
        ",".join(str(m) for m in safari_have) or "none",
        safari_host,
        "fill" if safari_gap else "ok",
        f"WKWebView dump {safari_gap}" if safari_gap else "current",
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
    print("suite is chrome, firefox, safari, edge. no brave.")


def fill_chrome(dry: bool, no_wire: bool) -> int:
    live_maj, live_ver = live_chrome_major()
    missing = missing_majors("chrome", live_maj)
    if not missing:
        print("chrome: current")
        return 0
    print(f"chrome: fill {missing} (live {live_ver})")
    if dry:
        return 0
    OUT.mkdir(parents=True, exist_ok=True)
    for major in missing:
        ver, binary = chrome_shell_for_major(major)
        dump = OUT / f"chrome-{major}.peet.json"
        print(f"capturing Chrome {ver}")
        peet = dump_chrome_headless(binary, dump)
        land_from_peet(
            "chrome",
            major,
            peet,
            captured=f"chrome-headless-shell-{ver}",
            verified=f"tls.peet.ws {TODAY} chrome-headless-shell {ver}",
        )
    if not no_wire:
        wire_family("chrome", "Chrome")
    return 0


def fill_firefox(dry: bool, no_wire: bool) -> int:
    live = live_firefox_version()
    live_maj = int(live.split(".", 1)[0])
    missing = missing_majors("firefox", live_maj)
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


def fill_safari(dry: bool, no_wire: bool) -> int:
    host = safari_host_version()
    if not host[:1].isdigit():
        print("safari: no Safari.app")
        return 1
    major = int(host.split(".", 1)[0])
    if major in bundled_majors("safari"):
        print(f"safari: current ({host})")
        return 0
    print(f"safari: fill {major} (host {host})")
    if dry:
        return 0
    pkg = ROOT / "tools/webkit-capture"
    print("safari: building webkit-probe")
    subprocess.check_call(["swift", "build", "-c", "release", "--package-path", str(pkg)])
    bins = [p for p in (pkg / ".build").rglob("webkit-probe") if p.is_file() and "dSYM" not in str(p)]
    if not bins:
        raise SystemExit("webkit-probe build produced no binary")
    probe = bins[0]
    OUT.mkdir(parents=True, exist_ok=True)
    dump = OUT / f"safari-{major}.peet.json"
    print(f"safari: WKWebView {PEET_URL}")
    raw = subprocess.check_output([str(probe), PEET_URL], timeout=45)
    text = raw.decode("utf-8", errors="replace")
    m = re.search(r"(\{.*\})", text, re.S)
    if not m:
        raise SystemExit(f"safari dump was not JSON: {text[:200]!r}")
    peet = json.loads(m.group(1))
    dump.write_text(json.dumps(peet))
    land_from_peet(
        "safari",
        major,
        peet,
        captured=f"webkit-{host}",
        verified=f"tls.peet.ws {TODAY} WKWebView Safari {host}",
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
    args = parser.parse_args(argv)
    CACHE.mkdir(parents=True, exist_ok=True)
    OUT.mkdir(parents=True, exist_ok=True)

    rows = catalog()
    print_catalog(rows)
    if args.target == "status":
        return 1 if any(r[3] == "fill" for r in rows) else 0

    rc = 0
    if args.target in {"all", "chrome"}:
        rc |= fill_chrome(args.dry_run, args.no_wire)
    if args.target in {"all", "firefox"}:
        rc |= fill_firefox(args.dry_run, args.no_wire)
    if args.target in {"all", "safari"}:
        rc |= fill_safari(args.dry_run, args.no_wire)
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
