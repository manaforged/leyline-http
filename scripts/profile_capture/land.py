from __future__ import annotations

import re
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

from .captures import load_h3, load_tcp
from .config import FAMILIES, OS_LABELS, PROFILES, TODAY
from .h3 import apply_h3, qpack
from .peet import chrome_grease, ciphers_from_peet, names_from_peet_ext, peet_get
from .toml_text import current, replace_list, replace_value, set_meta


@dataclass
class Landed:
    path: Path
    previous: int
    oses: list[str] = field(default_factory=list)
    notes: list[str] = field(default_factory=list)
    review: list[str] = field(default_factory=list)
    h3_oses: list[str] = field(default_factory=list)
    qpack: tuple[int, int] | None = None


def bundled_majors(family: str) -> list[int]:
    majors: list[int] = []
    for path in (PROFILES / family).glob("*.toml"):
        try:
            majors.append(int(path.stem))
        except ValueError:
            continue
    return sorted(majors)


def skeleton_toml(family: str, major: int) -> Path:
    lower = [m for m in bundled_majors(family) if m < major]
    if not lower:
        raise SystemExit(f"no {family} profile older than {major} to start from")
    return PROFILES / family / f"{lower[-1]}.toml"


def missing_majors(family: str, live_major: int) -> list[int]:
    have = set(bundled_majors(family))
    if not have:
        return [live_major]
    return [m for m in range(min(have) + 1, live_major + 1) if m not in have]


def os_phrase(names: list[str]) -> str:
    labels = [label for key, label in OS_LABELS.items() if key in names]
    if len(labels) < 3:
        return " and ".join(labels)
    return ", ".join(labels[:-1]) + ", and " + labels[-1]


def bump_versions(text: str, old: int, new: int) -> str:
    def bump(line: re.Match) -> str:
        value = re.sub(rf"(?<![\d.]){old}\.0", f"{new}.0", line.group(0))
        return value.replace(f'v="{old}"', f'v="{new}"')

    return re.sub(r"(?m)^(user_agent|sec_ch_ua) = .*$", bump, text)


def tcp_values(capture: dict) -> dict[str, object]:
    return {
        "ja4": peet_get(capture, "tls", "ja4"),
        "akamai": peet_get(capture, "http2", "akamai_fingerprint"),
        "ciphers": ciphers_from_peet(capture),
        "curves": names_from_peet_ext(capture, "supported_groups") or names_from_peet_ext(capture, "curves"),
        "sigalgs": names_from_peet_ext(capture, "signature_algorithms")
        or names_from_peet_ext(capture, "signature_algs"),
        "extensions": [
            int(ext["name"].rsplit("(", 1)[1].rstrip(")"))
            for ext in capture["tls"].get("extensions", [])
            if "GREASE" not in ext.get("name", "") and "(" in ext.get("name", "")
        ],
    }


def land(family: str, major: int, captured: str, date: str = TODAY) -> Landed:
    spec = FAMILIES[family]
    label = spec["label"]
    source = skeleton_toml(family, major)
    previous = int(source.stem)
    text = source.read_text()
    before = tomllib.loads(text)
    tcp = load_tcp(captured)
    if not tcp:
        raise SystemExit(f"no TCP capture named {captured}-<os>.json in the captures directory")
    oses = [key for key in OS_LABELS if key in tcp]
    values = [tcp_values(tcp[key]) for key in oses]
    landed = Landed(path=PROFILES / family / f"{major}.toml", previous=previous, oses=oses)
    for key in ("ja4", "akamai", "ciphers", "curves", "sigalgs"):
        if any(value[key] != values[0][key] for value in values[1:]):
            landed.review.append(f"the {key} differs between the {os_phrase(oses)} captures")
    seen = values[0]

    text = set_meta(text, "name", f'"{label} {major}"')
    text = set_meta(text, "version", str(major))
    text = set_meta(text, "variant", f'"{label}{major}"')
    text = set_meta(text, "hello", None)
    if "chromium_major" in before["meta"]:
        text = set_meta(text, "chromium_major", str(major))
    text = set_meta(text, "captured_against", f'"{captured}"')
    text = set_meta(text, "verified_at", f'"{date}"')

    for key in ("ciphers", "curves", "sigalgs"):
        if seen[key] and seen[key] != current(text, "tls", key):
            text = replace_list(text, "tls", key, seen[key])
            landed.review.append(f"[tls] {key} changed from {label} {previous}")
    if seen["ja4"] != before["tls"]["fingerprint"]["ja4"]:
        text = replace_value(text, "tls.fingerprint", "ja4", f'"{seen["ja4"]}"')
        landed.review.append("JA4 changed; check the extension order and resumed_ja4 by hand")
    if seen["akamai"] != before["h2"]["fingerprint"]["akamai"]:
        text = replace_value(text, "h2.fingerprint", "akamai", f'"{seen["akamai"]}"')
        landed.review.append("the HTTP/2 fingerprint changed; check [h2] by hand")
    order = before["tls"].get("extension_permutation")
    if order and seen["extensions"] and seen["extensions"] != order:
        landed.review.append("the TLS extension order changed; check extension_permutation by hand")

    text = bump_versions(text, previous, major)
    if family == "chrome":
        sch = chrome_grease(tcp[oses[0]], major)
        text = re.sub(r"(?m)^sec_ch_ua = .*$", lambda _: f"sec_ch_ua = '{sch}'", text)

    runs = load_h3(captured)
    if "h3" in before and spec["h3"] and runs:
        previous_id = before["meta"].get("captured_against", "")
        text, notes, review = apply_h3(text, runs, load_h3(previous_id))
        landed.notes += notes
        landed.review += review
        landed.h3_oses = sorted({path_os(run) for run in runs})
        landed.qpack = qpack(runs[0])
    elif "h3" in before:
        landed.review.append(f"no HTTP/3 capture; [h3] is a copy of {label} {previous}")

    verified = f"tls.peet.ws {date} {label} {captured.removeprefix(family + '-')} {os_phrase(oses)} {spec['tcp_method']}"
    if landed.h3_oses:
        verified += f"; quic.browserleaks.com {os_phrase(landed.h3_oses)} headful"
    text = set_meta(text, "verified_against", f'"{verified}"')
    landed.path.write_text(text)
    return landed


def path_os(run: dict) -> str:
    agent = run.get("user_agent", "")
    for key, marker in (("linux", "Linux"), ("macos", "Mac OS"), ("windows", "Windows")):
        if marker in agent:
            return key
    raise SystemExit(f"cannot tell the OS of an HTTP/3 capture from {agent!r}")
