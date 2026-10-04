from __future__ import annotations

import re
import tomllib


def section_span(text: str, section: str) -> tuple[int, int]:
    header = re.search(rf"(?m)^\[{re.escape(section)}\]\n", text)
    if not header:
        raise SystemExit(f"profile has no [{section}] section")
    after = re.search(r"(?m)^\[", text[header.end():])
    return header.end(), header.end() + after.start() if after else len(text)


def key_span(text: str, section: str, key: str) -> tuple[int, int]:
    start, end = section_span(text, section)
    match = re.search(rf"(?m)^{re.escape(key)} = (\[[^\]]*\]|.*)$", text[start:end])
    if not match:
        raise SystemExit(f"[{section}] has no {key}")
    return start + match.start(), start + match.end()


def current(text: str, section: str, key: str) -> object:
    start, end = key_span(text, section, key)
    return tomllib.loads(text[start:end])[key]


def replace_value(text: str, section: str, key: str, value: str) -> str:
    start, end = key_span(text, section, key)
    return f"{text[:start]}{key} = {value}{text[end:]}"


def render_list(items: list[str]) -> str:
    return "[\n" + "".join(f'    "{item}",\n' for item in items) + "]"


def replace_list(text: str, section: str, key: str, items: list[str]) -> str:
    old = current(text, section, key)
    if old == items:
        return text
    start, end = key_span(text, section, key)
    if [item for item in old if item in items] != items:
        return f"{text[:start]}{key} = {render_list(items)}{text[end:]}"
    block = text[start:end]
    for removed in (item for item in old if item not in items):
        quoted = re.escape(f'"{removed}"')
        block = re.sub(rf"{quoted},[ ]?|,[ ]*{quoted}(?=\s*\])|{quoted}", "", block, count=1)
    block = "\n".join(line.rstrip() for line in block.splitlines() if line.strip())
    edited = text[:start] + block + text[end:]
    if current(edited, section, key) != items:
        return f"{text[:start]}{key} = {render_list(items)}{text[end:]}"
    return edited


def set_meta(text: str, key: str, value: str | None) -> str:
    start, end = section_span(text, "meta")
    meta = text[start:end]
    line = re.compile(rf"(?m)^{re.escape(key)} = .*\n")
    if value is None:
        meta = line.sub("", meta, count=1)
    elif line.search(meta):
        meta = line.sub(lambda _: f"{key} = {value}\n", meta, count=1)
    else:
        meta, count = re.subn(r"(?m)^(version = \d+\n)", rf"\g<1>{key} = {value}\n", meta, count=1)
        if count != 1:
            raise SystemExit(f"[meta] has no version line to anchor {key}")
    return text[:start] + meta + text[end:]
