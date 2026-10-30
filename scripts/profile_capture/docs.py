from __future__ import annotations

import re
import sys
import textwrap
import tomllib
from pathlib import Path

from .config import BRANDS, BROWSER_IDS, FAMILIES, PROFILES, QPACK_GOLDEN, ROOT
from .land import build_label, profile_name

TABLE = ROOT / "docs/guide/bundled-profiles.md"
CHANGELOG = ROOT / "CHANGELOG.md"


CHANGELOG_SECTIONS = ("Security", "Added", "Changed", "Deprecated", "Removed", "Fixed")


def profile_rows() -> list[dict]:
    families = tomllib.loads((PROFILES / "families.toml").read_text())["family"]
    order = [browser for family in families for browser in family["browsers"]]
    rows = []
    for path in PROFILES.glob("*/*.toml"):
        profile = tomllib.loads(path.read_text())
        meta = profile.get("meta", {})
        if "variant" in meta:
            gated = "extension_permutation" in profile.get("tls", {})
            rows.append((order.index(meta["browser"]), meta["version"], {**meta, "gated": gated}))
    return [meta for _, _, meta in sorted(rows, key=lambda row: row[:2])]


def render_profiles() -> str:
    lines = [
        "| Profile | `Browser` variant | `capture` | `captured_against` | JA4 | Verified against |",
        "| --- | --- | --- | --- | --- | --- |",
    ]
    for meta in profile_rows():
        lines.append(
            f"| {meta['name']} | `{meta['variant']}` | `{meta['capture']}` | `{meta.get('captured_against', '')}` "
            f"| {'gated' if meta['gated'] else 'estimated'} | {meta.get('verified_against', '')} |"
        )
    chrome = sorted(meta["version"] for meta in profile_rows() if meta["browser"] == "chrome")
    lines += ["", "| Browser | `Browser` variants | Chrome capture |", "| --- | --- | --- |"]
    for brand, entry in tomllib.loads(BRANDS.read_text()).items():
        if not entry.get("ua_token"):
            continue
        if entry.get("follow_chromium"):
            bases = chrome
            own = chrome
        else:
            versions = entry.get("versions", {})
            bases = sorted(int(major) for major in versions if int(major) in chrome)
            own = sorted({int(build.split(".")[0]) for major in bases for build in versions[str(major)]})
        lines.append(
            f"| {brand} | `{brand}{own[0]}` to `{brand}{own[-1]}` | Chrome {bases[0]} to {bases[-1]} |"
        )
    return "\n".join(lines) + "\n"


def write_profiles() -> bool:
    text = render_profiles()
    if TABLE.exists() and TABLE.read_text() == text:
        return False
    TABLE.write_text(text)
    return True


def check_profiles() -> int:
    if TABLE.exists() and TABLE.read_text() == render_profiles():
        return 0
    print(f"{TABLE.relative_to(ROOT)} is stale: run scripts/profile-oneshot.sh docs", file=sys.stderr)
    return 1


def changelog_entry(text: str, family: str, major: int, version: str, phrase: str) -> str:
    label = FAMILIES[family]["label"]
    marker = f"`Browser::{label}{major}`"
    if marker in text:
        return text
    bullet = textwrap.fill(
        f"- A {profile_name(family, major)} profile, {marker}, from captures of {label} {version} on {phrase}.",
        width=79,
        subsequent_indent="  ",
        break_on_hyphens=False,
    )
    return changelog_add(text, "Added", bullet)


def changelog_add(text: str, section: str, bullet: str) -> str:
    if not re.search(r"(?m)^## Unreleased\n", text):
        first = re.search(r"(?m)^## ", text)
        if not first:
            raise SystemExit("CHANGELOG.md has no release heading")
        text = text[: first.start()] + "## Unreleased\n\n" + text[first.start() :]
    unreleased = re.search(r"(?m)^## Unreleased\n", text)
    end = re.search(r"(?m)^## ", text[unreleased.end() :])
    stop = unreleased.end() + end.start() if end else len(text)
    found = re.search(rf"(?m)^### {section}\n\n", text[unreleased.end() : stop])
    if found:
        at = unreleased.end() + found.end()
        return text[:at] + bullet + "\n" + text[at:]
    later = "|".join(CHANGELOG_SECTIONS[CHANGELOG_SECTIONS.index(section) + 1 :])
    after = re.search(rf"(?m)^### ({later})\n", text[unreleased.end() : stop]) if later else None
    at = unreleased.end() + after.start() if after else stop
    return text[:at] + f"### {section}\n\n{bullet}\n\n" + text[at:]


def profile_has_h3(family: str, major: int) -> bool:
    return "h3" in tomllib.loads((PROFILES / family / f"{major}.toml").read_text())


def previous_qpack(family: str, previous: int) -> tuple[int, int] | None:
    match = re.search(rf'(?m)^"{re.escape(family)}-{previous}" = \[(\d+), (\d+)\]$', QPACK_GOLDEN.read_text())
    return (int(match.group(1)), int(match.group(2))) if match else None


def qpack_row(text: str, family: str, major: int, previous: int, values: tuple[int, int]) -> str:
    key = f'"{family}-{major}"'
    if re.search(rf"(?m)^{re.escape(key)} = ", text):
        return text
    row = f"{key} = [{values[0]}, {values[1]}]"
    after = re.search(rf'(?m)^"{re.escape(family)}-{previous}" = .*\n', text)
    if not after:
        return text.rstrip("\n") + f"\n{row}\n"
    return text[: after.end()] + row + "\n" + text[after.end():]


def browser_id_row(text: str, variant: str) -> str:
    ids = tomllib.loads(text)
    if variant in ids:
        return text
    return text.rstrip("\n") + f"\n{variant} = {max(ids.values(), default=-1) + 1}\n"


def update(path: Path, edit) -> bool:
    before = path.read_text()
    after = edit(before)
    if after != before:
        path.write_text(after)
    return after != before


def document(family: str, major: int, previous: int, captured: str, phrase: str,
             qpack: tuple[int, int] | None) -> tuple[list[str], list[str]]:
    version = build_label(family, captured)
    changed, review = [], []
    if qpack is None and profile_has_h3(family, major):
        qpack = previous_qpack(family, previous)
        review.append(
            f"the QPACK golden row for {family}-{major} is a copy of {family}-{previous}"
            if qpack
            else f"add a QPACK golden row for {family}-{major} to {QPACK_GOLDEN.relative_to(ROOT)}"
        )
    label = FAMILIES[family]["label"]
    if update(BROWSER_IDS, lambda t: browser_id_row(t, f"{label}{major}")):
        changed.append(str(BROWSER_IDS.relative_to(ROOT)))
    if write_profiles():
        changed.append(str(TABLE.relative_to(ROOT)))
    if update(CHANGELOG, lambda t: changelog_entry(t, family, major, version, phrase)):
        changed.append(str(CHANGELOG.relative_to(ROOT)))
    if qpack and update(QPACK_GOLDEN, lambda t: qpack_row(t, family, major, previous, qpack)):
        changed.append(str(QPACK_GOLDEN.relative_to(ROOT)))
    return changed, review
