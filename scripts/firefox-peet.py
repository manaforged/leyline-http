#!/usr/bin/env python3
"""Drive Firefox via Marionette and print the JSON a fingerprint page returns.

Firefox has no Chrome-style --dump-dom. --marionette is the control channel.
"""
from __future__ import annotations

import json
import os
import signal
import socket
import subprocess
import sys
import tempfile
import time


def read_msg(sock: socket.socket) -> object:
    buf = b""
    while b":" not in buf:
        chunk = sock.recv(1)
        if not chunk:
            raise RuntimeError("marionette closed")
        buf += chunk
    n_s, rest = buf.split(b":", 1)
    n = int(n_s)
    while len(rest) < n:
        rest += sock.recv(n - len(rest))
    return json.loads(rest.decode())


def send_msg(sock: socket.socket, msgid: int, name: str, params: dict) -> object:
    payload = json.dumps([0, msgid, name, params]).encode()
    sock.sendall(str(len(payload)).encode() + b":" + payload)
    reply = read_msg(sock)
    if not isinstance(reply, list) or len(reply) != 4:
        raise RuntimeError(f"bad marionette reply: {reply!r}")
    _typ, _id, err, result = reply
    if err:
        raise RuntimeError(f"{name}: {err}")
    return result


def wait_port(host: str, port: int, timeout: float) -> None:
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with socket.create_connection((host, port), timeout=1):
                return
        except OSError:
            time.sleep(0.15)
    raise RuntimeError(f"marionette {host}:{port} did not come up")


def main() -> int:
    headful = "--headful" in sys.argv
    args = [arg for arg in sys.argv[1:] if arg != "--headful"]
    ff = args[0]
    url = args[1] if len(args) > 1 else "https://tls.peet.ws/api/all"
    out = args[2] if len(args) > 2 else ""
    port = int(os.environ.get("MARIONETTE_PORT", "29228"))
    profile = tempfile.mkdtemp(prefix="leyline-ff-")
    with open(os.path.join(profile, "user.js"), "w", encoding="utf-8") as fh:
        fh.write(f'user_pref("marionette.port", {port});\n')
        fh.write('user_pref("marionette.enabled", true);\n')
        fh.write('user_pref("datareporting.policy.dataSubmissionEnabled", false);\n')
        fh.write('user_pref("toolkit.telemetry.reportingpolicy.firstRun", false);\n')
        fh.write('user_pref("browser.shell.checkDefaultBrowser", false);\n')
        fh.write('user_pref("devtools.jsonview.enabled", false);\n')

    proc = subprocess.Popen(
        [
            ff,
            *([] if headful else ["--headless"]),
            "--marionette",
            "--profile",
            profile,
            "--remote-allow-hosts",
            "127.0.0.1",
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        start_new_session=True,
    )
    try:
        wait_port("127.0.0.1", port, 25)
        sock = socket.create_connection(("127.0.0.1", port), timeout=20)
        sock.settimeout(30)
        hello = read_msg(sock)
        if not isinstance(hello, dict):
            raise RuntimeError(f"expected marionette hello, got {hello!r}")
        send_msg(sock, 1, "WebDriver:NewSession", {"acceptInsecureCerts": True})
        send_msg(sock, 2, "WebDriver:Navigate", {"url": url})
        body = send_msg(
            sock,
            3,
            "WebDriver:ExecuteScript",
            {"script": "return document.body ? document.body.innerText : document.documentElement.textContent;", "args": []},
        )
        text = body.get("value") if isinstance(body, dict) else body
        if not isinstance(text, str) or "{" not in text:
            raise RuntimeError(f"page was not JSON: {text!r:.200}")
        start = text.find("{")
        end = text.rfind("}")
        obj = json.loads(text[start : end + 1])
        blob = json.dumps(obj)
        if out:
            with open(out, "w", encoding="utf-8") as fh:
                fh.write(blob)
                fh.write("\n")
        else:
            sys.stdout.write(blob)
            sys.stdout.write("\n")
        return 0
    finally:
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except OSError:
            pass
        proc.wait(timeout=5)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        sys.stderr.write(f"firefox-peet: {exc}\n")
        raise SystemExit(1)
