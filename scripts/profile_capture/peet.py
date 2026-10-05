from __future__ import annotations

import functools
import json
import re

from .config import ROOT

def peet_get(obj: dict, *keys: str) -> str:
    cur: object = obj
    for key in keys:
        if not isinstance(cur, dict):
            return ""
        cur = cur.get(key)
    return cur if isinstance(cur, str) else ""


def extract_json_blob(text: str, source: str) -> dict:
    m = re.search(r"(\{.*\})", text, re.S)
    if not m:
        raise SystemExit(f"{source} was not JSON: {text[:200]!r}")
    return json.loads(m.group(1))


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


@functools.cache
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
