from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

from .config import BUNDLE_KEY_ENV, BUNDLE_SKIP, BUNDLE_SOURCE, ED25519_SPKI_PREFIX, PROFILES, ROOT


def source() -> dict:
    return tomllib.loads(BUNDLE_SOURCE.read_text())


def asset_name() -> str:
    return source()["url"].rsplit("/", 1)[1]


def entries() -> list[dict]:
    rows = []
    for family in sorted(PROFILES.iterdir()):
        if not family.is_dir() or family.name in BUNDLE_SKIP:
            continue
        for path in sorted(family.glob("*.toml")):
            text = path.read_text()
            meta = tomllib.loads(text)["meta"]
            rows.append({"browser": meta["browser"], "version": meta["version"], "toml": text})
    return sorted(rows, key=lambda row: (row["browser"], row["version"]))


def generated() -> str:
    cmd = ["git", "-C", str(ROOT), "log", "-1", "--format=%cI", "--", str(PROFILES)]
    return subprocess.run(cmd, check=True, capture_output=True, text=True).stdout.strip()


def canonical(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()


def payload() -> bytes:
    return canonical({"format": source()["format"], "generated": generated(), "profiles": entries()})


def openssl(args: list[str]) -> subprocess.CompletedProcess:
    return subprocess.run(["openssl", "pkeyutl", *args], capture_output=True, check=False)


def signature_of(body: bytes) -> bytes:
    key = os.environ.get(BUNDLE_KEY_ENV)
    if not key:
        raise SystemExit(f"{BUNDLE_KEY_ENV} is not set")
    with tempfile.TemporaryDirectory() as scratch:
        pem, message, signature = (Path(scratch) / name for name in ("key.pem", "payload", "signature"))
        pem.touch(mode=0o600)
        pem.write_text(key)
        message.write_bytes(body)
        result = openssl(["-sign", "-rawin", "-inkey", str(pem), "-in", str(message), "-out", str(signature)])
        if result.returncode != 0:
            raise SystemExit(f"openssl could not sign the bundle: {result.stderr.decode().strip()}")
        return signature.read_bytes()


def sign(out: Path) -> Path:
    body = payload()
    envelope = {"payload": base64.b64encode(body).decode(), "signature": signature_of(body).hex()}
    path = out / asset_name()
    path.write_bytes(canonical(envelope))
    return path


def verify(path: Path) -> str:
    envelope = json.loads(path.read_bytes())
    with tempfile.TemporaryDirectory() as scratch:
        der, message, signature = (Path(scratch) / name for name in ("key.der", "payload", "signature"))
        message.write_bytes(base64.b64decode(envelope["payload"]))
        signature.write_bytes(bytes.fromhex(envelope["signature"]))
        for key in source()["public_keys"]:
            der.write_bytes(ED25519_SPKI_PREFIX + bytes.fromhex(key))
            args = ["-verify", "-pubin", "-keyform", "DER", "-inkey", str(der), "-rawin"]
            if openssl([*args, "-in", str(message), "-sigfile", str(signature)]).returncode == 0:
                return key
    raise SystemExit(f"{path.name} matches no key in {BUNDLE_SOURCE.relative_to(ROOT)}")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(prog="profile_capture.bundle")
    parser.add_argument("command", choices=["sign", "verify"])
    parser.add_argument("out", type=Path)
    args = parser.parse_args(argv)
    args.out.mkdir(parents=True, exist_ok=True)
    path = sign(args.out) if args.command == "sign" else args.out / asset_name()
    key = verify(path)
    body = base64.b64decode(json.loads(path.read_bytes())["payload"])
    print(f"{path.name}: signed by {key[:16]}, payload sha256 {hashlib.sha256(body).hexdigest()}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
