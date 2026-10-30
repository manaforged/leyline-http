#!/usr/bin/env python3
from __future__ import annotations

import argparse
import subprocess
import sys
import textwrap
import tomllib
from dataclasses import dataclass
from enum import IntEnum
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent
ROOT = SCRIPTS.parent
sys.path.insert(0, str(SCRIPTS))

from profile_capture.chrome import boringssl_revision, live_chrome_major
from profile_capture.docs import changelog_add

CRATE = ROOT / "crates/leyline-bssl-sys"
SUBMODULE = CRATE / "deps/boringssl"
SUBMODULE_REL = "crates/leyline-bssl-sys/deps/boringssl"
PATCHES = CRATE / "patches"
PATCH_GLOB = "*.patch"
APPLY_ARGS = ["apply", "-v", "--whitespace=fix"]
REJECT_ARGS = ["apply", "--reject", "--whitespace=fix"]
PIN = CRATE / "boringssl.toml"
CHANGELOG = ROOT / "CHANGELOG.md"
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


def git(cwd: Path, *args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(["git", "-C", str(cwd), *args], capture_output=True, text=True, check=check)


def bundled() -> Pin:
    have = git(ROOT, "ls-tree", "HEAD", SUBMODULE_REL).stdout.split()[2]
    pin = tomllib.loads(PIN.read_text())
    if pin["revision"] != have:
        raise SystemExit(f"{PIN.relative_to(ROOT)} names {pin['revision']}, the submodule is at {have}")
    return Pin(have, pin["chromium_tag"])


def wanted(revision: str | None) -> Pin:
    tag = live_chrome_major()[1]
    return Pin(revision or boringssl_revision(tag), tag)


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


def changelog_bullet(new: Pin, patches: int) -> str:
    return textwrap.fill(
        f"- BoringSSL is pinned to `{new.revision[:SHORT]}`, the `boringssl_revision` in "
        f"Chromium's DEPS at tag `{new.tag}`, and carries {patches} patches.",
        width=WRAP,
        subsequent_indent="  ",
        break_on_hyphens=False,
    )


def update_docs(old: Pin, new: Pin) -> list[Path]:
    PIN.write_text(f'revision = "{new.revision}"\nchromium_tag = "{new.tag}"\n')
    changed = [PIN]
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
    old = bundled()
    new = wanted(args.revision)
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
    report(old, new, update_docs(old, new))
    return Exit.BUMPED


if __name__ == "__main__":
    sys.exit(main())
