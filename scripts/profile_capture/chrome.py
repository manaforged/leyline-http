from __future__ import annotations

import hashlib
import json
import os
import platform as plat
import re
import select
import shutil
import signal
import subprocess
import tempfile
import time
import urllib.request
from pathlib import Path

from .captures import store, tcp_path
from .config import CACHE, CHROME_DEB_BASE, CHROME_DEB_INDEX, CHROME_STABLE_URL, HOST_OS, MAC_CHROME, OUT, PEET_URL, WAITS
from .land import missing_majors
from .net import http_bytes, http_json
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
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not select.select([self.recv_fd], [], [], remaining)[0]:
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


def page_text_via_cdp(cmd: list[str], url: str, errp: Path, timeout: float = WAITS["page_load"]) -> str:
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
            time.sleep(WAITS["page_poll"])
        raise SystemExit(f"{url} did not load within {timeout}s")
    finally:
        try:
            cdp.call("Browser.close", {}, time.monotonic() + WAITS["browser_close"])
        except (SystemExit, OSError):
            pass
        try:
            proc.wait(timeout=WAITS["browser_exit"])
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
    obj = extract_json_blob(text, f"Chrome {version} page")
    require_browser_ua(obj, f"Chrome/{version.split('.', 1)[0]}", f"Chrome {version}")
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
    with urllib.request.urlopen(CHROME_DEB_INDEX, timeout=WAITS["metadata_fetch"]) as resp:
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
    data = http_json(CHROME_STABLE_URL)
    ver = data["channels"]["Stable"]["version"]
    return int(ver.split(".", 1)[0]), ver


def fill_chrome(dry: bool, majors: list[int] | None = None) -> list[tuple[str, int, str]]:
    live_maj, live_ver = live_chrome_major()
    missing = sorted(set(majors)) if majors else missing_majors("chrome", live_maj)
    if not missing:
        print("chrome: current")
        return []
    print(f"chrome: fill {missing} (live {live_ver})")
    if dry:
        return []
    OUT.mkdir(parents=True, exist_ok=True)
    captured = []
    for major in missing:
        ver, binary = chrome_for_major(major)
        print(f"capturing Chrome {ver} from {binary}")
        peet = dump_chrome(binary, ver, OUT / f"chrome-{major}.peet.json")
        store(tcp_path(f"chrome-{ver}", HOST_OS[plat.system()]), peet, h3=False)
        captured.append(("chrome", major, f"chrome-{ver}"))
    return captured
