from __future__ import annotations

import ipaddress
import json
import re
from pathlib import Path

from .binaries import numeric_key
from .config import CAPTURES, KEPT_IP_KEYS, OS_LABELS, ROOT, TCP_SUFFIXES

IPV4 = re.compile(r"(?<![\d.])(?<![A-Za-z]/)(?:\d{1,3}\.){3}\d{1,3}(?!\d|\.\d)")
IPV6 = re.compile(r"(?<![\w:])[0-9a-fA-F]{0,4}(?::[0-9a-fA-F]{0,4}){2,7}(?![\w:])")


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
    for token in IPV4.findall(value) + IPV6.findall(value):
        try:
            address = ipaddress.ip_address(token)
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


def tcp_stem(stem: str) -> tuple[str, str, int] | None:
    for rank, suffix in enumerate(TCP_SUFFIXES):
        found = re.fullmatch(rf"(.+)-({'|'.join(OS_LABELS)}){re.escape(suffix)}", stem)
        if found:
            return found.group(1), found.group(2), rank
    return None


def load_tcp(captured: str) -> dict[str, dict]:
    found: dict[str, tuple[int, Path]] = {}
    for path in CAPTURES.glob(f"{captured}-*.json"):
        parsed = tcp_stem(path.stem)
        if parsed and parsed[0] == captured and (parsed[1] not in found or parsed[2] < found[parsed[1]][0]):
            found[parsed[1]] = (parsed[2], path)
    return {os_name: json.loads(path.read_text()) for os_name, (_, path) in sorted(found.items())}


def load_h3(captured: str) -> list[dict]:
    return [json.loads(path.read_text()) for path in sorted(CAPTURES.glob(f"{captured}-*-h3-run*.json"))]


def version_key(build: str) -> list[int]:
    return numeric_key(build.split("-", 1)[1])


def stored_builds(family: str, major: int) -> list[str]:
    newest: dict[str, str] = {}
    for path in CAPTURES.glob(f"{family}-{major}.*-*.json"):
        parsed = tcp_stem(path.stem)
        if not parsed:
            continue
        build, os_name, _ = parsed
        if os_name not in newest or version_key(build) > version_key(newest[os_name]):
            newest[os_name] = build
    return sorted(set(newest.values()), key=version_key)
