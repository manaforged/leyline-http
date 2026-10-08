#!/usr/bin/env python3
from __future__ import annotations

import argparse
import importlib.util
import json
import re
import subprocess
import sys
import textwrap
from dataclasses import dataclass
from enum import IntEnum
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent
ROOT = SCRIPTS.parent
sys.path.insert(0, str(SCRIPTS))

from profile_capture.docs import changelog_add

CRATE = ROOT / "crates/leyline-bssl-sys"
SUBMODULE = CRATE / "deps/boringssl"
SUBMODULE_REL = "crates/leyline-bssl-sys/deps/boringssl"
PATCHES = CRATE / "patches"
PATCH_GLOB = "*.patch"
APPLY_ARGS = ["apply", "-v", "--whitespace=fix"]
REJECT_ARGS = ["apply", "--reject", "--whitespace=fix"]
RELEASE_CHECK = SCRIPTS / "release-check.py"
PROVENANCE = CRATE / "PROVENANCE.md"
SECURITY = ROOT / "SECURITY.md"
CHANGELOG = ROOT / "CHANGELOG.md"
NOTICES = (ROOT / "NOTICE", CRATE / "NOTICE")
README = ROOT / "crates/leyline/README.md"
UPSTREAM_BASE = "e2a57cfb4d915b4ba820585aef9fdee7bca13fe5"
COMPARE_URL = "https://api.github.com/repos/google/boringssl/compare/{base}...{head}"
REGEN_COMMAND = "scripts/regen-bssl-bindings.sh <target>"
BINDINGS = CRATE / "bindings"
CHANGELOG_SECTION = "Changed"
WRAP = 79
SHORT = 7


class Exit(IntEnum):
    BUMPED = 0
    CURRENT = 0
    PATCH_FAILED = 3


@dataclass(frozen=True)
class Pin:
    revision: str
    tag: str

    @property
    def major(self) -> int:
        return int(self.tag.split(".")[0])


def release_check():
    spec = importlib.util.spec_from_file_location("release_check", RELEASE_CHECK)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def git(cwd: Path, *args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(["git", "-C", str(cwd), *args], capture_output=True, text=True, check=check)


def bundled() -> Pin:
    have = git(ROOT, "ls-tree", "HEAD", SUBMODULE_REL).stdout.split()[2]
    tag = re.search(r"DEPS at tag `([0-9.]+)`", PROVENANCE.read_text()).group(1)
    return Pin(have, tag)


def wanted(rc, revision: str | None) -> Pin:
    tag = rc.chrome_release()
    return Pin(revision or rc.boringssl_revision(tag), tag)


def move_submodule(revision: str) -> None:
    if git(SUBMODULE, "fetch", "--depth", "1", "origin", revision, check=False).returncode:
        git(SUBMODULE, "fetch", "origin")
    git(SUBMODULE, "checkout", "--detach", revision)


def restore(revision: str) -> None:
    git(SUBMODULE, "reset", "--hard")
    git(SUBMODULE, "clean", "-fdq")
    git(SUBMODULE, "checkout", "--detach", revision)


def rejected_hunks(patch: Path) -> str:
    git(SUBMODULE, "reset", "--hard")
    git(SUBMODULE, "clean", "-fdq")
    for earlier in sorted(PATCHES.glob(PATCH_GLOB)):
        if earlier == patch:
            break
        git(SUBMODULE, *APPLY_ARGS, str(earlier))
    git(SUBMODULE, *REJECT_ARGS, str(patch), check=False)
    found = git(SUBMODULE, "ls-files", "--others", "--exclude-standard").stdout.split()
    return "\n".join(
        f"--- {name}\n{(SUBMODULE / name).read_text()}" for name in found if name.endswith(".rej")
    )


def failing_patch() -> tuple[Path, str] | None:
    try:
        for patch in sorted(PATCHES.glob(PATCH_GLOB)):
            result = git(SUBMODULE, *APPLY_ARGS, str(patch), check=False)
            if result.returncode:
                return patch, result.stderr + "\n" + rejected_hunks(patch)
        return None
    finally:
        git(SUBMODULE, "reset", "--hard")
        git(SUBMODULE, "clean", "-fdq")


def commits_after_base(rc, revision: str) -> int:
    url = COMPARE_URL.format(base=UPSTREAM_BASE, head=revision)
    return json.loads(rc.fetch(url))["ahead_by"]


def rewrite(path: Path, edits: list[tuple[str, str]]) -> bool:
    before = path.read_text()
    after = before
    for pattern, replacement in edits:
        after = re.sub(pattern, replacement, after)
    if after == before:
        return False
    path.write_text(after)
    return True


def doc_edits(old: Pin, new: Pin, ahead: int) -> dict[Path, list[tuple[str, str]]]:
    pin = [(re.escape(old.revision), new.revision), (re.escape(f"tag `{old.tag}`"), f"tag `{new.tag}`")]
    chromium = [(rf"Chromium {old.major}'s DEPS", f"Chromium {new.major}'s DEPS")]
    edits = {
        PROVENANCE: pin + [(r"(`" + UPSTREAM_BASE + r"`, )\d+(\s+commits older)", rf"\g<1>{ahead}\g<2>")],
        SECURITY: pin,
        README: [(rf"revision Chrome {old.major} ships", f"revision Chrome {new.major} ships")],
    }
    edits.update({path: chromium for path in NOTICES})
    return edits


def changelog_bullet(new: Pin, patches: int) -> str:
    return textwrap.fill(
        f"- BoringSSL is pinned to `{new.revision[:SHORT]}`, the `boringssl_revision` in "
        f"Chromium's DEPS at tag `{new.tag}`, and carries {patches} patches.",
        width=WRAP,
        subsequent_indent="  ",
        break_on_hyphens=False,
    )


def update_docs(rc, old: Pin, new: Pin) -> list[Path]:
    ahead = commits_after_base(rc, new.revision)
    changed = [path for path, edits in doc_edits(old, new, ahead).items() if rewrite(path, edits)]
    bullet = changelog_bullet(new, len(list(PATCHES.glob(PATCH_GLOB))))
    text = CHANGELOG.read_text()
    updated = changelog_add(text, CHANGELOG_SECTION, bullet)
    if updated != text:
        CHANGELOG.write_text(updated)
        changed.append(CHANGELOG)
    return changed


def report(old: Pin, new: Pin, changed: list[Path]) -> None:
    targets = sorted(path.stem for path in BINDINGS.glob("*.rs"))
    print(f"BoringSSL {old.revision} (Chrome {old.tag}) -> {new.revision} (Chrome {new.tag})")
    print("Files changed:")
    for path in [SUBMODULE, *changed]:
        print(f"- {path.relative_to(ROOT)}")
    print("Regenerate the bindings for each target with a Rust toolchain:")
    for target in targets:
        print(f"  {REGEN_COMMAND.replace('<target>', target)}")


def main() -> int:
    parser = argparse.ArgumentParser(description="Move the bundled BoringSSL to Chrome stable's revision.")
    parser.add_argument("--revision", help="BoringSSL commit to pin instead of Chrome stable's")
    args = parser.parse_args()
    rc = release_check()
    old = bundled()
    new = wanted(rc, args.revision)
    if new.revision == old.revision:
        print("current")
        return Exit.CURRENT
    move_submodule(new.revision)
    failure = failing_patch()
    if failure:
        patch, detail = failure
        restore(old.revision)
        print(f"{patch.name} does not apply to BoringSSL {new.revision} (Chrome {new.tag})")
        print(detail)
        return Exit.PATCH_FAILED
    report(old, new, update_docs(rc, old, new))
    return Exit.BUMPED


if __name__ == "__main__":
    sys.exit(main())
