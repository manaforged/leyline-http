from __future__ import annotations

import base64
import json
import os
import socket
import struct
import subprocess
import time
from pathlib import Path

from .config import WAITS
from .proc import spawn, stop_tree

STDERR_LINES = 20
PAGE_TEXT = "document.readyState === 'complete' && document.body ? document.body.innerText : ''"


class Socket:
    def __init__(self, port: int, path: str, timeout: float) -> None:
        self.sock = socket.create_connection(("127.0.0.1", port), timeout=timeout)
        key = base64.b64encode(os.urandom(16)).decode()
        self.sock.sendall(
            f"GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n".encode()
        )
        self.buf = b""
        while b"\r\n\r\n" not in self.buf:
            self.fill(len(self.buf) + 1)
        head, self.buf = self.buf.split(b"\r\n\r\n", 1)
        if b" 101 " not in head.split(b"\r\n", 1)[0]:
            raise SystemExit(f"DevTools refused the WebSocket upgrade: {head[:80]!r}")
        self.next_id = 0

    def fill(self, size: int) -> None:
        while len(self.buf) < size:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise SystemExit("the browser closed the DevTools socket")
            self.buf += chunk

    def send(self, text: str) -> None:
        data, mask = text.encode(), os.urandom(4)
        size = len(data)
        if size < 126:
            head = struct.pack(">BB", 0x81, 0x80 | size)
        elif size < 65536:
            head = struct.pack(">BBH", 0x81, 0x80 | 126, size)
        else:
            head = struct.pack(">BBQ", 0x81, 0x80 | 127, size)
        self.sock.sendall(head + mask + bytes(byte ^ mask[i % 4] for i, byte in enumerate(data)))

    def receive(self) -> str:
        message = b""
        while True:
            self.fill(2)
            final, size, offset = self.buf[0] & 0x80, self.buf[1] & 0x7F, 2
            if size == 126:
                self.fill(4)
                size, offset = struct.unpack(">H", self.buf[2:4])[0], 4
            elif size == 127:
                self.fill(10)
                size, offset = struct.unpack(">Q", self.buf[2:10])[0], 10
            self.fill(offset + size)
            message += self.buf[offset : offset + size]
            self.buf = self.buf[offset + size :]
            if final:
                return message.decode()

    def call(self, method: str, params: dict, session: str | None = None) -> dict:
        self.next_id += 1
        message: dict = {"id": self.next_id, "method": method, "params": params}
        if session:
            message["sessionId"] = session
        self.send(json.dumps(message))
        while True:
            reply = json.loads(self.receive())
            if reply.get("id") != self.next_id:
                continue
            if "error" in reply:
                raise SystemExit(f"DevTools {method} failed: {reply['error']}")
            return reply.get("result") or {}


def devtools_port(profile: Path, proc: subprocess.Popen) -> tuple[int, str]:
    active = profile / "DevToolsActivePort"
    deadline = time.monotonic() + WAITS["driver_ready"]
    while time.monotonic() < deadline:
        lines = active.read_text().split() if active.is_file() else []
        if len(lines) == 2:
            return int(lines[0]), lines[1]
        if proc.poll() is not None:
            raise SystemExit(f"the browser exited with status {proc.returncode} before DevTools started")
        time.sleep(WAITS["driver_poll"])
    raise SystemExit(f"DevTools did not start within {WAITS['driver_ready']}s")


def page_text(cmd: list[str], profile: Path, url: str, errp: Path) -> str:
    with errp.open("wb") as err:
        proc = spawn(
            [*cmd, f"--user-data-dir={profile}", "--remote-debugging-port=0", "about:blank"],
            stdin=subprocess.DEVNULL,
            stdout=err,
            stderr=err,
        )
    try:
        return read_page(proc, profile, url)
    except SystemExit as failure:
        stop_tree(proc, WAITS["browser_exit"])
        tail = errp.read_text(errors="replace").strip().splitlines()[-STDERR_LINES:] if errp.is_file() else []
        raise SystemExit("\n".join([str(failure), *tail])) from None
    finally:
        stop_tree(proc, WAITS["browser_exit"])


def read_page(proc: subprocess.Popen, profile: Path, url: str) -> str:
    port, path = devtools_port(profile, proc)
    devtools = Socket(port, path, WAITS["page_load"])
    target = devtools.call("Target.createTarget", {"url": url})["targetId"]
    session = devtools.call("Target.attachToTarget", {"targetId": target, "flatten": True})["sessionId"]
    deadline = time.monotonic() + WAITS["page_load"]
    while time.monotonic() < deadline:
        result = devtools.call("Runtime.evaluate", {"expression": PAGE_TEXT, "returnByValue": True}, session)
        text = (result.get("result") or {}).get("value") or ""
        if text.strip().startswith("{"):
            return text
        time.sleep(WAITS["page_poll"])
    raise SystemExit(f"{url} did not load within {WAITS['page_load']}s")
