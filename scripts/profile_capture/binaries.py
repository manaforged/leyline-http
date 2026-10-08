from __future__ import annotations

import functools
import hashlib
import os
import platform as plat
import plistlib
import re
import shutil
import subprocess
import tarfile
from pathlib import Path

from .config import (
    CACHE,
    CHROMIUM_BUILDS,
    FF_ARCHIVES,
    FF_DOWNLOAD_URL,
    GITHUB_TOKEN_ENV,
    HOST_OS,
    OPERA,
    SIGNERS,
)
from .net import http_bytes, http_json, http_text

Build = tuple[str, Path]


def host() -> str:
    return HOST_OS[plat.system()]


def major_of(version: str) -> int:
    return int(version.split(".", 1)[0])


def powershell(script: str, path: Path) -> str:
    return subprocess.check_output(
        ["powershell", "-NoProfile", "-Command", script.replace("{path}", str(path).replace("'", "''"))],
        text=True,
    ).strip()


def verify_signer(family: str, path: Path) -> None:
    expected = SIGNERS[family][host()]
    if host() == "macos":
        subprocess.run(["codesign", "--verify", "--deep", "--strict", str(path)], check=True)
        details = subprocess.run(["codesign", "-dv", str(path)], capture_output=True, text=True, check=True).stderr
        found = re.search(r"(?m)^TeamIdentifier=(\S+)$", details)
        signer = found.group(1) if found else ""
        if signer != expected:
            raise SystemExit(f"{path} is signed by team {signer!r}, not {expected!r}; refusing to capture")
    elif host() == "windows":
        status, _, subject = powershell(
            "$s = Get-AuthenticodeSignature -LiteralPath '{path}'; \"$($s.Status)|$($s.SignerCertificate.Subject)\"",
            path,
        ).partition("|")
        if status != "Valid" or expected not in subject:
            raise SystemExit(f"{path} signature is {status} by {subject!r}, not {expected!r}; refusing to capture")


def copy_from_dmg(dmg: Path, app_name: str, dest: Path) -> Path:
    attached = subprocess.check_output(["hdiutil", "attach", "-nobrowse", "-readonly", str(dmg)], text=True)
    mounts = [line.split("\t")[-1].strip() for line in attached.splitlines() if "/Volumes/" in line]
    if not mounts:
        raise SystemExit(f"hdiutil did not mount {dmg}")
    try:
        version = plistlib.loads((Path(mounts[0]) / app_name / "Contents/Info.plist").read_bytes())[
            "CFBundleShortVersionString"
        ]
        target = dest.with_name(dest.name.format(ver=version)) / app_name
        if not target.exists():
            shutil.copytree(Path(mounts[0]) / app_name, target, symlinks=True)
        return target
    finally:
        subprocess.run(["hdiutil", "detach", mounts[0]], check=False, stdout=subprocess.DEVNULL)


def chromium_version(family: str, binary: Path) -> str:
    product = CHROMIUM_BUILDS[family]["product"]
    if "headless-shell" in str(binary).lower():
        raise SystemExit(f"{binary} is chrome-headless-shell; capture from the full {product} build")
    if host() == "windows":
        out = powershell("$v = (Get-Item -LiteralPath '{path}').VersionInfo; \"$($v.ProductName) $($v.ProductVersion)\"", binary)
    else:
        out = subprocess.check_output([str(binary), "--version"], text=True).strip()
    found = re.fullmatch(rf"{re.escape(product)} (\d+\.\d+\.\d+\.\d+)(?: \S+)?", out)
    if not found:
        raise SystemExit(f"{binary} reports {out!r}, not {product}; refusing to capture")
    return found.group(1)


def numeric_key(version: str) -> list[int]:
    return [int(part) for part in re.findall(r"\d+", version)]


def verified_download(url: str, dest: Path, sha256: str | None) -> None:
    print(f"downloading {url}")
    http_bytes(url, dest)
    if sha256 is not None and hashlib.sha256(dest.read_bytes()).hexdigest() != sha256.lower():
        dest.unlink()
        raise SystemExit(f"{url}: download does not match its published SHA-256")


def extract_deb(url: str, sha256: str, directory: Path) -> None:
    deb = CACHE / url.rsplit("/", 1)[1]
    verified_download(url, deb, sha256)
    subprocess.run(["dpkg-deb", "-x", str(deb), str(directory)], check=True)
    deb.unlink()


def apt_package(base: str, index: str, package: str) -> dict[str, str]:
    found = []
    for block in http_text(base + index).split("\n\n"):
        fields = dict(line.split(": ", 1) for line in block.splitlines() if ": " in line)
        if fields.get("Package") == package:
            found.append(fields)
    if not found:
        raise SystemExit(f"{package} is missing from {base + index}")
    return max(found, key=lambda fields: numeric_key(fields["Version"]))


@functools.cache
def chromium_release(family: str) -> dict[str, object]:
    spec = CHROMIUM_BUILDS[family]["release"]
    token = os.environ.get(GITHUB_TOKEN_ENV)
    release = http_json(spec["url"], headers={"Authorization": f"Bearer {token}"} if token else None)
    found = re.fullmatch(spec["name"], (release.get("name") or "").strip())
    if release.get("prerelease") or not found:
        raise SystemExit(f"latest {family} release {release.get('name')!r} is not a stable release")
    return {"chromium": int(found.group("chromium")), "version": found.group("version"), "name": release["name"],
            "assets": release.get("assets") or []}


def download_source(family: str, os_name: str) -> tuple[str, str | None]:
    spec = CHROMIUM_BUILDS[family][os_name]
    if "url" in spec:
        return spec["url"], None
    asset = next((a for a in chromium_release(family)["assets"] if a["name"] == spec["asset"]), None)
    digest = (asset or {}).get("digest") or ""
    if not digest.startswith("sha256:"):
        raise SystemExit(f"{family} release has no {spec['asset']} with a SHA-256 digest")
    return asset["browser_download_url"], digest.removeprefix("sha256:")


def linux_chromium(family: str) -> Path:
    apt = CHROMIUM_BUILDS[family]["apt"]
    fields = apt_package(apt["base"], apt["index"], apt["package"])
    directory = CACHE / f"{family}-{fields['Version']}-linux"
    binary = directory / apt["binary"]
    if not binary.is_file():
        extract_deb(apt["base"] + fields["Filename"], fields["SHA256"], directory)
    return binary


def macos_chromium(family: str) -> Path:
    spec = CHROMIUM_BUILDS[family]["macos"]
    url, sha256 = download_source(family, "macos")
    dmg = CACHE / f"{family}.dmg"
    verified_download(url, dmg, sha256)
    app = copy_from_dmg(dmg, spec["app"], CACHE / f"{family}-{{ver}}-macos")
    dmg.unlink()
    verify_signer(family, app)
    return app / spec["exe"]


def installed(paths: list[str]) -> Path | None:
    return next((Path(os.path.expandvars(p)) for p in paths if Path(os.path.expandvars(p)).is_file()), None)


def windows_chromium(family: str, major: int) -> Path:
    spec = CHROMIUM_BUILDS[family]["windows"]
    binary = installed(spec["paths"])
    if binary is None or major_of(chromium_version(family, binary)) != major:
        url, sha256 = download_source(family, "windows")
        setup = CACHE / url.rsplit("/", 1)[1]
        verified_download(url, setup, sha256)
        verify_signer(family, setup)
        subprocess.run([part.replace("{installer}", str(setup)) for part in spec["install"]], check=True)
        setup.unlink()
        binary = installed(spec["paths"])
        if binary is None:
            raise SystemExit(f"{CHROMIUM_BUILDS[family]['product']} is not at any of {spec['paths']} after install")
    verify_signer(family, binary)
    return binary


def chromium_binary(family: str, major: int) -> Build:
    spec = CHROMIUM_BUILDS[family]
    override = os.environ.get(spec["env"])
    if override:
        binary = Path(override)
    elif host() == "windows":
        binary = windows_chromium(family, major)
    else:
        binary = {"linux": linux_chromium, "macos": macos_chromium}[host()](family)
    ver = chromium_version(family, binary)
    if major_of(ver) != major:
        raise SystemExit(f"{spec['product']} {major}: {binary} is {ver}; the official build serves only current stable")
    return ver, binary


def opera_linux(ver: str) -> Path:
    directory = CACHE / f"opera-{ver}-linux"
    binary = directory / OPERA["binary"]
    if not binary.is_file():
        url = OPERA["index"] + OPERA["deb"].format(ver=ver)
        extract_deb(url, http_text(url + OPERA["checksum"]).split()[0], directory)
    return binary


def firefox_linux(ver: str) -> Path:
    if plat.machine().lower() not in {"x86_64", "amd64"}:
        raise SystemExit("Firefox collection on Linux requires x86_64")
    directory = CACHE / f"firefox-{ver}-linux-x86_64"
    binary = directory / "firefox" / "firefox"
    if not binary.is_file():
        archive = CACHE / f"firefox-{ver}-linux-x86_64.tar.xz"
        print(f"downloading Firefox {ver}")
        http_bytes(FF_DOWNLOAD_URL + FF_ARCHIVES["linux"].format(ver=ver), archive)
        with tarfile.open(archive) as bundle:
            bundle.extractall(directory, filter="data")
        archive.unlink()
    return binary


def firefox_macos(ver: str) -> Path:
    app = CACHE / f"firefox-{ver}-macos" / "Firefox.app"
    if not app.exists():
        dmg = CACHE / f"Firefox-{ver}.dmg"
        print(f"downloading Firefox {ver}")
        http_bytes(FF_DOWNLOAD_URL + FF_ARCHIVES["macos"].format(ver=ver), dmg)
        app = copy_from_dmg(dmg, "Firefox.app", CACHE / "firefox-{ver}-macos")
        dmg.unlink()
    verify_signer("firefox", app)
    return app / "Contents/MacOS/firefox"


def firefox_windows(ver: str) -> Path:
    directory = CACHE / f"firefox-{ver}-windows"
    binary = directory / "core" / "firefox.exe"
    if not binary.is_file():
        setup = CACHE / f"Firefox-Setup-{ver}.exe"
        print(f"downloading Firefox {ver}")
        http_bytes(FF_DOWNLOAD_URL + FF_ARCHIVES["windows"].format(ver=ver), setup)
        verify_signer("firefox", setup)
        subprocess.run([str(setup), f"/ExtractDir={directory}"], check=True)
        setup.unlink()
    verify_signer("firefox", binary)
    return binary


def firefox_binary(ver: str) -> Path:
    binary = {"linux": firefox_linux, "macos": firefox_macos, "windows": firefox_windows}[host()](ver)
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise SystemExit(f"Firefox executable missing at {binary}")
    return binary
