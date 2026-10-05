#!/usr/bin/env python3
from __future__ import annotations

import functools
import json
import os
import re
import subprocess
import sys
import tomllib
import urllib.error
import urllib.request
from pathlib import Path

from update_notice import notify

ROOT = Path(__file__).resolve().parent.parent
GITHUB = "https://api.github.com"
OSV = "https://api.osv.dev/v1/query"


def request(url: str, body: dict | None = None) -> object:
    headers = {"Accept": "application/vnd.github+json", "User-Agent": "leyline-upstream-check"}
    token = os.environ.get("GITHUB_TOKEN")
    if token and url.startswith(GITHUB):
        headers["Authorization"] = f"Bearer {token}"
    data = None
    if body is not None:
        data = json.dumps(body).encode()
        headers["Content-Type"] = "application/json"
    req = urllib.request.Request(url, data=data, headers=headers)
    with urllib.request.urlopen(req, timeout=30) as resp:
        return json.load(resp)


def version_key(text: str) -> tuple[int, ...]:
    return tuple(int(part) for part in re.findall(r"\d+", text)[:3])


def clause_ok(base: tuple[int, ...], clause: str) -> bool:
    clause = clause.strip()
    if clause.endswith("+"):
        clause = ">=" + clause[:-1]
    match = re.match(r"(>=|<=|>|<|=)?\s*v?([\d.]+)$", clause)
    if not match:
        raise ValueError(f"unparsed version range clause {clause!r}")
    op, bound = match.group(1) or "=", version_key(match.group(2))
    return {">=": base >= bound, "<=": base <= bound, ">": base > bound, "<": base < bound, "=": base == bound}[op]


def in_range(version: str, spec: str) -> bool:
    base = version_key(version)
    return any(
        all(clause_ok(base, clause) for clause in alternative.split(",") if clause.strip())
        for alternative in spec.split("||")
    )


def fixed_by(version: str, patched: list[str]) -> bool:
    if not patched:
        return False
    base = version_key(version)
    if base >= max(version_key(p) for p in patched):
        return True
    return any(version_key(p)[:2] == base[:2] and base >= version_key(p) for p in patched)


@functools.cache
def github_advisories(repo: str) -> list[dict]:
    return request(f"{GITHUB}/repos/{repo}/security-advisories?per_page=100&state=published")


@functools.cache
def latest(repo: str) -> str:
    try:
        return request(f"{GITHUB}/repos/{repo}/releases/latest")["tag_name"]
    except urllib.error.HTTPError as err:
        if err.code != 404:
            raise
    tags = request(f"{GITHUB}/repos/{repo}/tags?per_page=100")
    names = [tag["name"] for tag in tags if re.fullmatch(r"v?\d+\.\d+\.\d+", tag["name"])]
    return max(names, key=version_key) if names else "unknown"


def osv(body: dict) -> list[str]:
    return [vuln["id"] for vuln in request(OSV, body).get("vulns", [])]


def repo_hits(repo: str, crate: str, version: str) -> list[str]:
    hits = []
    for advisory in github_advisories(repo):
        for vuln in advisory.get("vulnerabilities") or []:
            name = (vuln.get("package") or {}).get("name")
            spec = vuln.get("vulnerable_version_range") or ""
            patched = [p for p in re.split(r"[,\s]+", vuln.get("patched_versions") or "") if p]
            names = (crate, None, repo.split("/")[-1], repo.replace("/", "-"))
            if name in names and spec and in_range(version, spec) and not fixed_by(version, patched):
                hits.append(advisory["ghsa_id"])
                break
    return hits


def check_crate(name: str, meta: dict) -> None:
    repo, crate, version = meta["repo"], meta["crate"], meta["version"]
    hits = repo_hits(repo, crate, version)
    hits += osv({"version": version, "package": {"name": crate, "ecosystem": "crates.io"}})
    hits = sorted(set(hits))
    newest = latest(repo)
    behind = version_key(newest) > version_key(version)
    status = "behind" if behind else "current"
    print(f"{name}: {repo} {crate} base {version} latest {newest} ({status}) advisories: {', '.join(hits) or 'none'}")
    if hits:
        notify(
            f"Security advisory for {name}: {', '.join(hits)}",
            f"{', '.join(hits)} applies to {repo} {crate} {version}, the base of {name}. "
            f"The latest {crate} is {newest}. Rebase {name} on a fixed release.",
        )


LISTING_CAP = 1000


def shared_advisories(meta: dict) -> list[str]:
    feed = meta.get("shared_advisories")
    if not feed:
        return []
    listing = request(f"{GITHUB}/repos/{feed['repo']}/contents/{feed['path']}")
    if len(listing) >= LISTING_CAP:
        raise RuntimeError(f"{feed['repo']}/{feed['path']} lists {len(listing)} files; the listing may be truncated")
    pattern = re.compile(r"(CVE-(\d+)-\d+)\.json")
    reviewed = set(meta.get("reviewed", {}))
    found = []
    for entry in listing:
        match = pattern.fullmatch(entry["name"])
        if not match or match.group(1) in reviewed or int(match.group(2)) < feed["id_year_floor"]:
            continue
        record = request(feed["record_url"].format(cve=match.group(1)))
        published = record["containers"]["cna"].get("datePublic", "")[:10]
        if not published or published >= feed["since"]:
            found.append(match.group(1))
    return sorted(found)


def check_boringssl(crate_dir: Path, meta: dict) -> None:
    repo, gitlink = meta["repo"], meta["gitlink"]
    rel = (crate_dir / gitlink).relative_to(ROOT).as_posix()
    tree = subprocess.run(["git", "-C", str(ROOT), "ls-tree", "HEAD", rel], capture_output=True, text=True, check=True)
    revision = tree.stdout.split()[2]
    compare = request(f"{GITHUB}/repos/{repo}/compare/{revision}...main")
    hits = osv({"commit": revision})
    shared = shared_advisories(meta)
    print(
        f"{crate_dir.name}: {repo} gitlink {revision[:12]} behind main by {compare['ahead_by']} commits"
        f" advisories: {', '.join(hits) or 'none'} shared to review: {', '.join(shared) or 'none'}"
    )
    feed = meta.get("shared_advisories", {}).get("repo")
    for cve in shared:
        notify(
            f"Review {cve} against the bundled BoringSSL",
            f"{feed} published {cve}. {crate_dir.name} bundles BoringSSL {revision[:12]}. "
            "BoringSSL shares code with OpenSSL. Check whether BoringSSL is affected. If it is, carry the fix "
            f"as a patch in {crate_dir.relative_to(ROOT).as_posix()}/patches/. Then add {cve} to `reviewed` "
            f"in {crate_dir.relative_to(ROOT).as_posix()}/Cargo.toml.",
        )
    if hits:
        notify(
            f"Security advisory for {crate_dir.name} BoringSSL: {', '.join(hits)}",
            f"{', '.join(hits)} applies to {repo} {revision}, the BoringSSL revision {crate_dir.name} bundles. "
            f"Move {rel} to a fixed revision.",
        )


def main() -> int:
    failed = False
    for manifest in sorted(ROOT.glob("crates/*/Cargo.toml")):
        package = tomllib.loads(manifest.read_text())["package"]
        meta = package.get("metadata", {}).get("upstream")
        if not meta:
            continue
        for check, args in ((check_crate, (package["name"], meta)), (check_boringssl, (manifest.parent, meta.get("boringssl")))):
            if args[1] is None:
                continue
            try:
                check(*args)
            except Exception as error:
                print(f"::error::{package['name']}: check failed: {error}")
                failed = True
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
