from __future__ import annotations

import ipaddress
import json
import re
from pathlib import Path

from .config import CAPTURES, KEPT_IP_KEYS, ROOT

IP_TEXT = re.compile(r"(?<![\w/.:-])[0-9a-fA-F:.]{7,}(?![\w.])")


def scrub(capture: dict, h3: bool) -> dict:
    def walk(value: object) -> object:
        if isinstance(value, dict):
            out = {}
            for key, item in value.items():
                if key == "src_ip":
                    out[key] = None if h3 else ""
                elif key == "iso_code":
                    out[key] = ""
                else:
                    out[key] = walk(item)
            return out
        if isinstance(value, list):
            return [walk(item) for item in value]
        return value

    cleaned = walk(capture)
    if not h3:
        cleaned.pop("ip", None)
    leaked = public_ips(cleaned)
    if leaked:
        raise SystemExit(f"capture still holds a client address under {sorted(leaked)}; refusing to store it")
    return cleaned


def public_ips(value: object, key: str = "") -> set[str]:
    if isinstance(value, dict):
        return set().union(*(public_ips(item, name) for name, item in value.items()))
    if isinstance(value, list):
        return set().union(*(public_ips(item, key) for item in value))
    if not isinstance(value, str) or key in KEPT_IP_KEYS:
        return set()
    found = set()
    for token in IP_TEXT.findall(value):
        host = token.rsplit(":", 1)[0] if token.count(":") == 1 else token
        try:
            address = ipaddress.ip_address(host)
        except ValueError:
            continue
        if address.is_global:
            found.add(key or "<value>")
    return found


def tcp_path(captured: str, os_name: str) -> Path:
    return CAPTURES / f"{captured}-{os_name}.json"


def h3_path(captured: str, os_name: str, run: int) -> Path:
    return CAPTURES / f"{captured}-{os_name}-h3-run{run}.json"


def store(path: Path, capture: dict, h3: bool) -> Path:
    path.write_text(json.dumps(scrub(capture, h3), indent=2))
    print(f"stored {path.relative_to(ROOT)}")
    return path


def load_tcp(captured: str) -> dict[str, dict]:
    found = {}
    for path in sorted(CAPTURES.glob(f"{captured}-*.json")):
        os_name = path.stem.removeprefix(f"{captured}-")
        if "-" not in os_name:
            found[os_name] = json.loads(path.read_text())
    return found


def load_h3(captured: str) -> list[dict]:
    return [json.loads(path.read_text()) for path in sorted(CAPTURES.glob(f"{captured}-*-h3-run*.json"))]
