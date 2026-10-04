from __future__ import annotations

import re
import textwrap
from pathlib import Path

from .config import FAMILIES, QPACK_GOLDEN, ROOT

GUIDE = ROOT / "docs/guide/profiles.md"
README = ROOT / "crates/leyline/README.md"
CHANGELOG = ROOT / "CHANGELOG.md"


def insert_after_row(text: str, previous_row: str, row: str) -> str:
    if row in text:
        return text
    match = re.search(rf"(?m)^{re.escape(previous_row)}.*\n", text)
    if not match:
        raise SystemExit(f"no table row starting {previous_row!r}")
    return text[: match.end()] + row + "\n" + text[match.end():]


def guide_rows(text: str, family: str, major: int, previous: int, captured: str, phrase: str) -> str:
    label = FAMILIES[family]["label"]
    method = FAMILIES[family]["tcp_method"]
    status = re.search(rf"(?m)^\| {label} {previous} \| `{label}{previous}` \| `[^`]*` \| (\w+) \|$", text)
    if not status:
        raise SystemExit(f"no {label} {previous} row in the profile table")
    text = insert_after_row(
        text,
        f"| {label} {previous} | `{label}{previous}` |",
        f"| {label} {major} | `{label}{major}` | `{captured}` | {status.group(1)} |",
    )
    return insert_after_row(
        text,
        f"| {label} {previous} | `browser` |",
        f"| {label} {major} | `browser` | `{captured}`, {phrase} `{method}` |",
    )


def readme_range(text: str, family: str, major: int, previous: int) -> str:
    label = FAMILIES[family]["label"]
    return re.sub(rf"(?m)^(\| {label} \| \d+ to ){previous}( \|)", rf"\g<1>{major}\g<2>", text, count=1)


def changelog_entry(text: str, family: str, major: int, version: str, phrase: str) -> str:
    label = FAMILIES[family]["label"]
    marker = f"`Browser::{label}{major}`"
    if marker in text:
        return text
    bullet = textwrap.fill(
        f"- A {label} {major} profile, {marker}, from captures of {label} {version} on {phrase}.",
        width=79,
        subsequent_indent="  ",
        break_on_hyphens=False,
    )
    unreleased = re.search(r"(?m)^## Unreleased\n", text)
    if not unreleased:
        raise SystemExit("CHANGELOG.md has no ## Unreleased section")
    end = re.search(r"(?m)^## ", text[unreleased.end():])
    stop = unreleased.end() + end.start() if end else len(text)
    added = re.search(r"(?m)^### Added\n\n", text[unreleased.end():stop])
    if added:
        at = unreleased.end() + added.end()
        return text[:at] + bullet + "\n" + text[at:]
    changed = re.search(r"(?m)^### (Changed|Fixed)\n", text[unreleased.end():stop])
    at = unreleased.end() + changed.start() if changed else stop
    return text[:at] + f"### Added\n\n{bullet}\n\n" + text[at:]


def qpack_row(text: str, family: str, major: int, previous: int, values: tuple[int, int]) -> str:
    key = f'"{family}-{major}"'
    if re.search(rf"(?m)^{re.escape(key)} = ", text):
        return text
    row = f"{key} = [{values[0]}, {values[1]}]"
    after = re.search(rf'(?m)^"{re.escape(family)}-{previous}" = .*\n', text)
    if not after:
        return text.rstrip("\n") + f"\n{row}\n"
    return text[: after.end()] + row + "\n" + text[after.end():]


def update(path: Path, edit) -> bool:
    before = path.read_text()
    after = edit(before)
    if after != before:
        path.write_text(after)
    return after != before


def document(family: str, major: int, previous: int, captured: str, phrase: str,
             qpack: tuple[int, int] | None) -> list[str]:
    version = captured.removeprefix(f"{family}-")
    changed = []
    if update(GUIDE, lambda t: guide_rows(t, family, major, previous, captured, phrase)):
        changed.append(str(GUIDE.relative_to(ROOT)))
    if update(README, lambda t: readme_range(t, family, major, previous)):
        changed.append(str(README.relative_to(ROOT)))
    if update(CHANGELOG, lambda t: changelog_entry(t, family, major, version, phrase)):
        changed.append(str(CHANGELOG.relative_to(ROOT)))
    if qpack and update(QPACK_GOLDEN, lambda t: qpack_row(t, family, major, previous, qpack)):
        changed.append(str(QPACK_GOLDEN.relative_to(ROOT)))
    return changed
