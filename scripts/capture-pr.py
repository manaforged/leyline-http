#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import textwrap
from pathlib import Path

from profile_capture.captures import stored_builds
from profile_capture.config import CHROMIUM_BUILDS, FAMILIES, OPERA, ROOT

ISSUE_NAMES = {"brave": "brave (chromium)"}
PROFILE_TEST = (
    "cargo test -p leyline-http --locked --features bench-internals --lib --test it --"
    " profile fingerprint_conformance:: → test result: ok"
)
BORINGSSL_TEST = "cargo test -p leyline-http --locked --features full,bench-internals → test result: ok"
BORINGSSL_PIN = re.compile(r"BoringSSL (\w+) \(Chrome ([\d.]+)\) -> (\w+) \(Chrome ([\d.]+)\)")
BRAND_ROW = re.compile(r'"(\d+)" has (\d+)')


def run(*args: str) -> str:
    return subprocess.run(args, cwd=ROOT, check=True, capture_output=True, text=True).stdout.strip()


def open_issues(prefix: str) -> list[int]:
    found = json.loads(run("gh", "issue", "list", "--state", "open", "--search", f'"{prefix}" in:title', "--json", "number,title"))
    return [issue["number"] for issue in found if issue["title"].startswith(prefix)]


def message(subject: str, labels: list[str]) -> str:
    body = "\n".join(textwrap.fill(line, width=72, break_on_hyphens=False) for line in labels)
    return f"{subject}\n\n{body}"


def body(summary: str, report: list[str], review: list[str], issues: list[int]) -> str:
    lines = [summary, ""] + [f"- {line}" for line in report]
    if review:
        lines += ["", "Review before merging:", ""] + [f"- [ ] {line}" for line in review]
    if issues:
        lines += [""] + [f"Closes #{number}." for number in issues]
    return "\n".join(lines) + "\n"


def publish(branch: str, title: str, commit: str, text: str) -> int:
    run("git", "switch", "-C", branch)
    run("git", "add", "-A")
    if not run("git", "status", "--porcelain"):
        print(f"{branch}: nothing to publish")
        return 0
    run("git", "commit", "-m", commit)
    run("git", "push", "--force", "origin", branch)
    open_prs = json.loads(run("gh", "pr", "list", "--head", branch, "--state", "open", "--json", "number"))
    if open_prs:
        run("gh", "pr", "edit", str(open_prs[0]["number"]), "--title", title, "--body", text)
    else:
        run("gh", "pr", "create", "--base", "main", "--head", branch, "--title", title, "--body", text)
    return 0


def landed_lines(lines: list[str]) -> list[str]:
    report: list[str] = []
    for line in lines:
        if line.startswith("landed "):
            report.append(line)
        elif report and line.startswith("  "):
            report.append(line.strip())
        elif report:
            break
    return report


def profile(family: str, major: int, lines: list[str]) -> int:
    label = FAMILIES[family]["label"]
    builds = stored_builds(family, major)
    report = landed_lines(lines)
    review = [line.removeprefix("review: ") for line in lines if line.startswith("review: ")]
    if family in CHROMIUM_BUILDS:
        review += [f"the BoringSSL update in #{number} lands with this profile" for number in open_issues("Update BoringSSL to ")]
    commit = message(
        f"profiles: add {family} {major}",
        [
            f"Problem: {label} {major} is the stable release, and Leyline has no profile for it.",
            f"Fix: Land the {label} {major} profile from the captures of {' and '.join(builds)}.",
            f"Impact: Browser::{label}{major} sends the fingerprint of the shipped build.",
            f"Test: {PROFILE_TEST}",
        ],
    )
    issues = open_issues(f"Add a {ISSUE_NAMES.get(family, family)} {major} profile")
    text = body(f"Adds the {label} {major} profile from captures of the shipped browser.", report, review, issues)
    return publish(f"capture/{family}-{major}", f"profiles: add {label} {major}", commit, text)


def brand(major: int, lines: list[str]) -> int:
    name = OPERA["brand"]
    row = next((found for found in map(BRAND_ROW.search, lines) if found), None)
    if not row:
        print(f"{name} {major}: no brand row was written")
        return 0
    chromium = row.group(1)
    commit = message(
        f"profiles: add the {name.lower()} {major} brand row",
        [
            f"Problem: {name} {major} is the stable release, and brands.toml has no row for it.",
            f"Fix: Map Chromium {chromium} to {name} {major}, from the user agent of the shipped build.",
            f"Impact: The {name} brand on Chrome {chromium} sends {name} {major}.",
            "Test: cargo test -p leyline-http --locked --lib -- profile::brand → test result: ok",
        ],
    )
    issues = open_issues(f"Add a {name.lower()} {major} profile")
    report = [line.strip() for line in lines if line.startswith("updated ")]
    text = body(f"Adds the {name} {major} brand row on Chromium {chromium}.", report, [], issues)
    return publish(f"capture/{name.lower()}-{major}", f"profiles: add the {name} {major} brand row", commit, text)


def boringssl(lines: list[str]) -> int:
    pin = next((found for found in map(BORINGSSL_PIN.search, lines) if found), None)
    if not pin:
        print("boringssl: current")
        return 0
    old, new, tag = pin.group(1), pin.group(3), pin.group(4)
    major = tag.split(".", 1)[0]
    commit = message(
        f"bssl-sys: build BoringSSL at the Chrome {major} revision",
        [
            f"Problem: BoringSSL is pinned to {old[:7]}, and Chrome {tag} ships {new[:7]}.",
            f"Fix: Pin {new[:7]}, apply the patches unchanged, and regenerate the bindings for each target.",
            f"Impact: The TLS stack is the one Chrome {major} ships.",
            f"Test: {BORINGSSL_TEST}",
        ],
    )
    report = [line.strip().removeprefix("- ") for line in lines if line.strip() and not line.rstrip().endswith(":")]
    text = body(f"Moves BoringSSL to the revision Chrome {tag} ships.", report, [], open_issues("Update BoringSSL to "))
    return publish(f"capture/boringssl-{new[:12]}", f"bssl-sys: build BoringSSL at the Chrome {major} revision", commit, text)


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="Open or update the pull request for a captured change.")
    parser.add_argument("kind", choices=["profile", "brand", "boringssl"])
    parser.add_argument("report", type=Path)
    parser.add_argument("--family")
    parser.add_argument("--major", type=int)
    args = parser.parse_args(argv)
    lines = args.report.read_text().splitlines()
    if args.kind == "profile":
        return profile(args.family, args.major, lines)
    if args.kind == "brand":
        return brand(args.major, lines)
    return boringssl(lines)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
