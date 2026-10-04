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


FINGERPRINT_KEYS = ("ja4", "akamai", "ciphers", "curves", "sigalgs")


@dataclass
class Draft:
    family: str
    major: int
    captured: str
    text: str
    before: dict
    tcp: dict[str, dict]
    oses: list[str]
    seen: dict[str, object]
    landed: Landed

    @property
    def label(self) -> str:
        return FAMILIES[self.family]["label"]


def read_captures(family: str, major: int, captured: str) -> Draft:
    source = skeleton_toml(family, major)
    tcp = load_tcp(captured)
    if not tcp:
        raise SystemExit(f"no TCP capture named {captured}-<os>.json in the captures directory")
    oses = [key for key in OS_LABELS if key in tcp]
    values = [tcp_values(tcp[key]) for key in oses]
    empty = [key for key in FINGERPRINT_KEYS if not all(value[key] for value in values)]
    if empty:
        raise SystemExit(f"{captured}: the capture has no {', '.join(empty)}; refusing to land it")
    landed = Landed(path=PROFILES / family / f"{major}.toml", previous=int(source.stem), oses=oses)
    landed.review += [
        f"the {key} differs between the {os_phrase(oses)} captures"
        for key in FINGERPRINT_KEYS
        if any(value[key] != values[0][key] for value in values[1:])
    ]
    text = source.read_text()
    return Draft(family, major, captured, text, tomllib.loads(text), tcp, oses, values[0], landed)


def land_meta(draft: Draft, date: str) -> None:
    text = draft.text
    for key, value in (
        ("name", f'"{draft.label} {draft.major}"'),
        ("version", str(draft.major)),
        ("variant", f'"{draft.label}{draft.major}"'),
        ("hello", None),
        ("captured_against", f'"{draft.captured}"'),
        ("verified_at", f'"{date}"'),
    ):
        text = set_meta(text, key, value)
    if "chromium_major" in draft.before["meta"]:
        text = set_meta(text, "chromium_major", str(draft.major))
    draft.text = text


def land_tls(draft: Draft) -> None:
    seen, before, review = draft.seen, draft.before, draft.landed.review
    for key in ("ciphers", "curves", "sigalgs"):
        if seen[key] != current(draft.text, "tls", key):
            draft.text = replace_list(draft.text, "tls", key, seen[key])
            review.append(f"[tls] {key} changed from {draft.label} {draft.landed.previous}")
    if seen["ja4"] != before["tls"]["fingerprint"]["ja4"]:
        draft.text = replace_value(draft.text, "tls.fingerprint", "ja4", f'"{seen["ja4"]}"')
        review.append("JA4 changed; check the extension order and resumed_ja4 by hand")
    if seen["akamai"] != before["h2"]["fingerprint"]["akamai"]:
        draft.text = replace_value(draft.text, "h2.fingerprint", "akamai", f'"{seen["akamai"]}"')
        review.append("the HTTP/2 fingerprint changed; check [h2] by hand")
    order = before["tls"].get("extension_permutation")
    if order and seen["extensions"] and seen["extensions"] != order:
        review.append("the TLS extension order changed; check extension_permutation by hand")


def land_identity(draft: Draft) -> None:
    first = draft.tcp[draft.oses[0]]
    draft.text = bump_versions(draft.text, draft.landed.previous, draft.major)
    if FAMILIES[draft.family]["ua_from_capture"]:
        agent = first.get("user_agent", "")
        draft.text = replace_value(draft.text, f"identity.{draft.oses[0]}", "user_agent", f'"{agent}"')
    if draft.family == "chrome":
        sch = chrome_grease(first, draft.major)
        draft.text = re.sub(r"(?m)^sec_ch_ua = .*$", lambda _: f"sec_ch_ua = '{sch}'", draft.text)
    draft.landed.review += [
        f"user agent still names another version: {agent}"
        for agent in re.findall(r'(?m)^user_agent = "([^"]*)"$', draft.text)
        if str(draft.major) not in agent
    ]


def land_h3(draft: Draft) -> None:
    if "h3" not in draft.before:
        return
    runs = load_h3(draft.captured)
    if not (FAMILIES[draft.family]["h3"] and runs):
        draft.landed.review.append(f"no HTTP/3 capture; [h3] is a copy of {draft.label} {draft.landed.previous}")
        return
    previous_runs = load_h3(draft.before["meta"].get("captured_against", ""))
    draft.text, notes, review = apply_h3(draft.text, runs, previous_runs)
    draft.landed.notes += notes
    draft.landed.review += review
    draft.landed.h3_oses = sorted({path_os(run) for run in runs})
    draft.landed.qpack = qpack(runs[0])


def verified_against(draft: Draft, date: str) -> str:
    method = FAMILIES[draft.family]["tcp_method"]
    version = draft.captured.removeprefix(draft.family + "-")
    verified = f"tls.peet.ws {date} {draft.label} {version} {os_phrase(draft.oses)} {method}"
    if draft.landed.h3_oses:
        verified += f"; quic.browserleaks.com {os_phrase(draft.landed.h3_oses)} headful"
    return verified


def land(family: str, major: int, captured: str, date: str = TODAY) -> Landed:
    draft = read_captures(family, major, captured)
    land_meta(draft, date)
    land_tls(draft)
    land_identity(draft)
    land_h3(draft)
    draft.text = set_meta(draft.text, "verified_against", f'"{verified_against(draft, date)}"')
    draft.landed.path.write_text(draft.text)
    return draft.landed


def path_os(run: dict) -> str:
    agent = run.get("user_agent", "")
    for key, marker in (("linux", "Linux"), ("macos", "Mac OS"), ("windows", "Windows")):
        if marker in agent:
            return key
    raise SystemExit(f"cannot tell the OS of an HTTP/3 capture from {agent!r}")
