import argparse
import json
import os
import re
from pathlib import Path
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]
BOOK = ROOT / "docs"
PAGES = BOOK / "reference"
PACKAGE = "leyline-http"
FEATURES = "full"
TOOLCHAIN = "nightly-2026-09-16"
HOST = "aarch64-apple-darwin"
PUBLIC_API_VERSION = "0.52.0"
SECTION = "# API reference"
PATH = re.compile(r"leyline(::[A-Za-z0-9_]+)*")


def run(command, **kwargs):
    return subprocess.check_output(command, cwd=ROOT, text=True, **kwargs)


def public_api(json_path):
    api = run(
        [
            "cargo-public-api",
            "--rustdoc-json",
            str(json_path),
            "--color",
            "never",
            "--include",
            "function-parameter-names",
            "--omit",
            "blanket-impls,auto-trait-impls,auto-derived-impls",
        ]
    ).rstrip()
    if not api:
        raise SystemExit(f"Empty API output for {PACKAGE}")
    return api


def item_path(line):
    match = PATH.search(line)
    return match.group(0) if match else ""


def module_sections(api):
    rows = api.splitlines()
    modules = [row.removeprefix("pub mod ") for row in rows if row.startswith("pub mod ")]
    groups = {module: [] for module in modules}
    for row in rows:
        path = item_path(row)
        owner = max(
            (m for m in modules if path == m or path.startswith(m + "::")),
            key=len,
            default=modules[0],
        )
        groups[owner].append(row)
    lines = []
    for module in modules:
        lines.extend([f"## `{module}`", ""])
        items = {}
        for row in groups[module]:
            rest = item_path(row).removeprefix(module).removeprefix("::")
            items.setdefault(rest.split("::", 1)[0], []).append(row)
        for name, block in sorted(items.items()):
            if name:
                lines.extend([f"### `{name}`", ""])
            lines.extend(["```rust,ignore", *block, "```", ""])
    return lines


def render(api, json_path, version):
    documented = json.loads(json_path.read_text())
    symbols = sorted(
        {
            item["name"]
            for item in documented["index"].values()
            if item["crate_id"] == 0 and item["name"] and item["name"] in api
        }
    )
    header = [
        f"# {PACKAGE}",
        "",
        "Every public item of the crate, generated from the compiler's view of the",
        f"code with `{version}`. Features: `{FEATURES}`. Target: `{HOST}`.",
        "Items marked `#[doc(hidden)]` are internal and not listed.",
        "",
        "<details>",
        "<summary>Symbol index</summary>",
        "",
        " · ".join(f"`{symbol}`" for symbol in symbols),
        "",
        "</details>",
        "",
    ]
    return "\n".join(header + module_sections(api))


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Generate the mdBook API reference from the compiler's public API."
    )
    parser.add_argument("--check", action="store_true", help="Fail when the pages are stale.")
    args = parser.parse_args()
    version = run(["cargo-public-api", "--version"]).strip()
    if version != f"cargo-public-api {PUBLIC_API_VERSION}":
        raise SystemExit(f"Run cargo install cargo-public-api --version {PUBLIC_API_VERSION} --locked.")
    cargo = ["rustup", "run", TOOLCHAIN, "cargo"]
    env = {"RUSTDOCFLAGS": "-Z unstable-options --output-format=json"}
    metadata = json.loads(run(cargo + ["metadata", "--locked", "--no-deps", "--format-version", "1"]))
    subprocess.run(
        cargo
        + ["doc", "--locked", "-p", PACKAGE, "--lib", "--no-deps", "--features", FEATURES, "--target", HOST],
        cwd=ROOT,
        env={**os.environ, **env},
        check=True,
    )
    package = next(p for p in metadata["packages"] if p["name"] == PACKAGE)
    lib = next(t for t in package["targets"] if "lib" in t["kind"])
    json_path = Path(metadata["target_directory"]) / HOST / "doc" / f"{lib['name']}.json"
    api = public_api(json_path)
    page = PAGES / f"{PACKAGE}.md"
    summary = BOOK / "SUMMARY.md"
    prefix = summary.read_text().split(SECTION, 1)[0].rstrip()
    pages = {
        page: render(api, json_path, version),
        summary: prefix + f"\n\n{SECTION}\n\n- [{PACKAGE}](reference/{PACKAGE}.md)\n",
    }
    stale = [path for path, content in pages.items() if not path.exists() or path.read_text() != content]
    if args.check:
        if stale:
            raise SystemExit("Stale API pages: " + ", ".join(str(p.relative_to(ROOT)) for p in stale))
        print(f"API reference is current: {len(api.splitlines())} items.")
        return
    for path, content in pages.items():
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)
    print(f"Wrote {page.relative_to(ROOT)}: {len(api.splitlines())} items.")


if __name__ == "__main__":
    try:
        main()
    except (FileNotFoundError, subprocess.CalledProcessError) as error:
        print(error, file=sys.stderr)
        raise SystemExit(1) from error
