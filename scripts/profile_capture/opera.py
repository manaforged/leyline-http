from __future__ import annotations

import re
import tomllib

from .binaries import major_of, numeric_key, opera_linux
from .chrome import dump_chrome
from .config import BRANDS, OPERA, PEET_URL, ROOT
from .net import http_text
from .toml_text import add_list_item


def live_opera_version() -> str:
    return max(re.findall(OPERA["version"], http_text(OPERA["index"])), key=numeric_key)


def opera_has(major: int) -> bool:
    rows = tomllib.loads(BRANDS.read_text())[OPERA["brand"]].get("versions", {})
    return any(major_of(value) == major for values in rows.values() for value in values)


def ua_major(pattern: str, agent: str, source: str) -> int:
    found = re.search(pattern, agent)
    if not found:
        raise SystemExit(f"{source}: user agent {agent!r} does not match {pattern!r}")
    return int(found.group(1))


def fill_opera(dry: bool) -> list[tuple[str, int, str]]:
    ver = live_opera_version()
    if opera_has(major_of(ver)):
        print("opera: current")
        return []
    print(f"opera: add a {OPERA['brand']} {major_of(ver)} brand row (live {ver})")
    if dry:
        return []
    source = f"{OPERA['brand']} {ver}"
    peet = dump_chrome(opera_linux(ver), source, PEET_URL, None, None)
    agent = peet.get("user_agent") or ""
    chromium = ua_major(OPERA["ua_chromium"], agent, source)
    brand = ua_major(OPERA["ua_brand"], agent, source)
    if brand != major_of(ver):
        raise SystemExit(f"{source}: user agent {agent!r} names {OPERA['brand']} {brand}")
    section = f"{OPERA['brand']}.versions"
    text = add_list_item(BRANDS.read_text(), section, f'"{chromium}"', OPERA["brand_version"].format(major=brand))
    BRANDS.write_text(text)
    print(f"updated {BRANDS.relative_to(ROOT)}: [{section}] \"{chromium}\" has {brand}")
    return []
