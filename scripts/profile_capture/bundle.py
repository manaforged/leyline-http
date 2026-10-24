from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import time
import tomllib
from pathlib import Path

from .config import (
    BUNDLE_FIXTURE_PROFILES,
    BUNDLE_FIXTURE_TIMES,
    BUNDLE_FIXTURES,
    BUNDLE_KEY_ENV,
    BUNDLE_SKIP,
    BUNDLE_SOURCE,
    ED25519_PKCS8_PREFIX,
    ED25519_SPKI_PREFIX,
    PROFILES,
    RFC8032_TEST1_SEED,
    ROOT,
    TLS_VERSIONS,
)


def source() -> dict:
    return tomllib.loads(BUNDLE_SOURCE.read_text())


def asset_name() -> str:
    return source()["url"].rsplit("/", 1)[1]


def below_floor(profile: dict) -> bool:
    floor = TLS_VERSIONS.index(source()["policy"]["min_tls_version"])
    blocks = [profile.get("tls", {}), profile.get("h3", {}).get("tls", {})]
    declared = [block["min_tls_version"] for block in blocks if "min_tls_version" in block]
    return any(TLS_VERSIONS.index(version) < floor for version in declared)


def entry(text: str) -> dict:
    meta = tomllib.loads(text)["meta"]
    return {"browser": meta["browser"], "version": meta["version"], "toml": text}


def entries() -> list[dict]:
    rows = []
    for family in sorted(PROFILES.iterdir()):
        if not family.is_dir() or family.name in BUNDLE_SKIP:
            continue
        for path in sorted(family.glob("*.toml")):
            text = path.read_text()
            if not below_floor(tomllib.loads(text)):
                rows.append(entry(text))
    return sorted(rows, key=lambda row: (row["browser"], row["version"]))


def profiles_changed_at() -> int:
    spec = [f":(glob){PROFILES.relative_to(ROOT)}/*/**"]
    spec += [f":(exclude){PROFILES.relative_to(ROOT)}/{name}" for name in sorted(BUNDLE_SKIP)]
    cmd = ["git", "-C", str(ROOT), "log", "-1", "--first-parent", "--format=%ct", "--", *spec]
    return int(subprocess.run(cmd, check=True, capture_output=True, text=True).stdout.strip())


def canonical(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()


def head() -> str:
    cmd = ["git", "-C", str(ROOT), "rev-parse", "HEAD"]
    return subprocess.run(cmd, check=True, capture_output=True, text=True).stdout.strip()


def payload(now: int) -> dict:
    lifetime = source()["max_lifetime_secs"]
    body = {"format": source()["format"], "serial": now, "expires": now + lifetime, "profiles": entries()}
    return {**body, "commit": head()}


def code_changes(since: str | None) -> list[str]:
    if not since:
        return ["no commit recorded in the published bundle"]
    paths = [str(PROFILES.relative_to(ROOT)), "scripts/profile_capture", "scripts/bundle-check", ".github/workflows"]
    cmd = ["git", "-C", str(ROOT), "diff", "--stat", f"{since}..HEAD", "--", *paths]
    result = subprocess.run(cmd, capture_output=True, text=True, check=False)
    if result.returncode != 0:
        return [f"commit {since} is not in this history"]
    return [f"since {since[:12]}: {line.strip()}" for line in result.stdout.splitlines()]


def openssl(args: list[str]) -> subprocess.CompletedProcess:
    return subprocess.run(["openssl", "pkeyutl", *args], capture_output=True, check=False)


def signature_of(body: bytes, key: str) -> bytes:
    with tempfile.TemporaryDirectory() as scratch:
        pem, message, signature = (Path(scratch) / name for name in ("key.pem", "payload", "signature"))
        pem.touch(mode=0o600)
        pem.write_text(key)
        message.write_bytes(body)
        result = openssl(["-sign", "-rawin", "-inkey", str(pem), "-in", str(message), "-out", str(signature)])
        if result.returncode != 0:
            raise SystemExit(f"openssl could not sign the bundle: {result.stderr.decode().strip()}")
        return signature.read_bytes()


def envelope(body: dict, key: str) -> bytes:
    raw = canonical(body)
    return canonical({"payload": base64.b64encode(raw).decode(), "signature": signature_of(raw, key).hex()})


def signing_key() -> str:
    key = os.environ.get(BUNDLE_KEY_ENV)
    if not key:
        raise SystemExit(f"{BUNDLE_KEY_ENV} is not set")
    return key


def sign(out: Path) -> Path:
    path = out / asset_name()
    path.write_bytes(envelope(payload(int(time.time())), signing_key()))
    return path


def verify(path: Path, keys: list[str]) -> str:
    sealed = json.loads(path.read_bytes())
    with tempfile.TemporaryDirectory() as scratch:
        der, message, signature = (Path(scratch) / name for name in ("key.der", "payload", "signature"))
        message.write_bytes(base64.b64decode(sealed["payload"]))
        signature.write_bytes(bytes.fromhex(sealed["signature"]))
        for key in keys:
            der.write_bytes(ED25519_SPKI_PREFIX + bytes.fromhex(key))
            args = ["-verify", "-pubin", "-keyform", "DER", "-inkey", str(der), "-rawin"]
            if openssl([*args, "-in", str(message), "-sigfile", str(signature)]).returncode == 0:
                return key
    raise SystemExit(f"{path.name} matches no trusted key")


def decode(path: Path) -> dict:
    return json.loads(base64.b64decode(json.loads(path.read_bytes())["payload"]))


def changes(old: list[dict], new: list[dict]) -> list[str]:
    before = {(row["browser"], row["version"]): row["toml"] for row in old}
    after = {(row["browser"], row["version"]): row["toml"] for row in new}
    keys = sorted(before.keys() | after.keys())
    marks = {(True, False): "removed", (False, True): "added"}
    return [f"{marks.get((key in before, key in after), 'changed')} {key[0]} {key[1]}" for key in keys if before.get(key) != after.get(key)]


def plan(published: Path) -> tuple[bool, list[str]]:
    fresh, now = payload(int(time.time())), int(time.time())
    if not published.is_file():
        return True, ["no published bundle", f"commit {fresh['commit']}"]
    try:
        verify(published, source()["public_keys"])
    except SystemExit as error:
        return True, [f"published bundle does not verify: {error}"]
    current = decode(published)
    diff = changes(current.get("profiles", []), fresh["profiles"])
    context = [f"commit {fresh['commit']}", *diff, *code_changes(current.get("commit"))]
    if current.get("format") != fresh["format"] or diff:
        return True, [f"format {current.get('format')} -> {fresh['format']}", *context]
    if current.get("serial", 0) < source()["min_serial"]:
        return True, [f"published serial {current.get('serial', 0)} is below min_serial {source()['min_serial']}", *context]
    left = current.get("expires", 0) - now
    if left < source()["signer"]["renew_secs"]:
        return True, [f"published bundle expires in {left} seconds", *context]
    return False, [f"published bundle is current until {current['expires']}"]


def test_key() -> tuple[str, str]:
    pem = base64.encodebytes(ED25519_PKCS8_PREFIX + RFC8032_TEST1_SEED).decode()
    key = f"-----BEGIN PRIVATE KEY-----\n{pem}-----END PRIVATE KEY-----\n"
    with tempfile.TemporaryDirectory() as scratch:
        path = Path(scratch) / "key.pem"
        path.write_text(key)
        der = subprocess.run(["openssl", "pkey", "-in", str(path), "-pubout", "-outform", "DER"], check=True, capture_output=True).stdout
    return key, der.removeprefix(ED25519_SPKI_PREFIX).hex()


def fixtures(out: Path) -> list[Path]:
    key, public = test_key()
    times = BUNDLE_FIXTURE_TIMES
    texts = [(PROFILES / browser / f"{version}.toml").read_text() for browser, version in BUNDLE_FIXTURE_PROFILES]
    broken = texts[-1].replace(f"version = {BUNDLE_FIXTURE_PROFILES[-1][1]}", "version = 999", 1).replace("[meta]\n", "[meta]\nfuture_field = true\n", 1)
    tls10 = texts[-1].replace("[tls]\n", '[tls]\nmin_tls_version = "1.0"\n', 1)
    tls12 = texts[-1].replace("[tls]\n", '[tls]\nmin_tls_version = "1.2"\n', 1)
    weak_cipher = texts[-1].replace("ciphers = [", 'ciphers = ["TLS_RSA_WITH_NULL_SHA256", ', 1)
    borrowed_cipher = texts[-1].replace("ciphers = [", 'ciphers = ["TLS_RSA_WITH_3DES_EDE_CBC_SHA", ', 1)
    head, tail = texts[-1].split('user_agent = "', 1)
    injected = f'{head}user_agent = "Mozilla/5.0\\r\\nX-Injected: 1{tail[tail.index(chr(34)):]}'
    profiles = [entry(text) for text in texts] + [{"browser": "chrome", "version": 999, "toml": broken}]
    base = {"format": 1, "serial": times["serial"], "expires": times["expires"], "profiles": profiles}
    bodies = {
        "two-chrome": base,
        "format-2": {**base, "format": 2},
        "rolled-back": {**base, "serial": times["older"]},
        "future-serial": {**base, "serial": times["future"]},
        "expired": {**base, "expires": times["expired"]},
        "tls10": {**base, "profiles": [entry(tls10)]},
        "tls12": {**base, "profiles": [entry(tls12)]},
        "mismatched-entry": {**base, "profiles": [{**entry(texts[-1]), "version": BUNDLE_FIXTURE_PROFILES[0][1]}]},
        "weak-cipher": {**base, "profiles": [entry(weak_cipher)]},
        "borrowed-cipher": {**base, "profiles": [entry(borrowed_cipher)]},
        "header-injection": {**base, "profiles": [entry(injected)]},
    }
    out.mkdir(parents=True, exist_ok=True)
    written = []
    for name, body in bodies.items():
        written.append(out / f"{name}.json")
        written[-1].write_bytes(envelope(body, key))
    sealed = json.loads((out / "two-chrome.json").read_bytes())
    raw = bytearray(base64.b64decode(sealed["payload"]))
    raw[len(raw) // 2] ^= 1
    written.append(out / "tampered.json")
    written[-1].write_bytes(canonical({**sealed, "payload": base64.b64encode(bytes(raw)).decode()}))
    written.append(out / "key.hex")
    written[-1].write_text(public + "\n")
    return written


def summary(path: Path) -> str:
    body = decode(path)
    raw = base64.b64decode(json.loads(path.read_bytes())["payload"])
    return f"{path.name}: serial {body['serial']}, expires {body['expires']}, {len(body['profiles'])} profiles, payload sha256 {hashlib.sha256(raw).hexdigest()}"


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(prog="profile_capture.bundle")
    parser.add_argument("command", choices=["sign", "verify", "plan", "fixtures", "floor"])
    parser.add_argument("path", type=Path, nargs="?", default=Path("."))
    args = parser.parse_args(argv)
    if args.command == "floor":
        floor, newest = source()["min_serial"], profiles_changed_at()
        print(f"min_serial {floor}, newest profile change {newest}")
        return 0 if floor == newest else 1
    if args.command == "fixtures":
        for path in fixtures(args.path):
            print(path.relative_to(ROOT) if path.is_relative_to(ROOT) else path)
        return 0
    if args.command == "plan":
        needed, reasons = plan(args.path / asset_name())
        report = "\n".join([f"{'sign' if needed else 'skip'}: {reasons[0]}", *(f"- {line}" for line in reasons[1:])])
        print(report)
        if summary_file := os.environ.get("GITHUB_STEP_SUMMARY"):
            with open(summary_file, "a") as handle:
                handle.write(report + "\n")
        if output := os.environ.get("GITHUB_OUTPUT"):
            with open(output, "a") as handle:
                handle.write(f"needed={str(needed).lower()}\ncommit={head()}\n")
        return 0
    if args.command == "sign":
        args.path.mkdir(parents=True, exist_ok=True)
        path = sign(args.path)
    else:
        path = args.path
    key = verify(path, source()["public_keys"])
    print(f"{summary(path)}, signed by {key[:16]}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
