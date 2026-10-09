from __future__ import annotations

import os
import tempfile
from datetime import date
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
PEET_URL = os.environ.get("PEET_URL", "https://tls.peet.ws/api/all")
FF_URL = "https://product-details.mozilla.org/1.0/firefox_versions.json"
FF_RELEASES_URL = "https://product-details.mozilla.org/1.0/firefox.json"
FF_DOWNLOAD_URL = os.environ.get("LEYLINE_FF_DOWNLOAD", "https://download-installer.cdn.mozilla.net/pub/firefox/releases/")
FF_ARCHIVES = {
    "linux": "{ver}/linux-x86_64/en-US/firefox-{ver}.tar.xz",
    "macos": "{ver}/mac/en-US/Firefox%20{ver}.dmg",
    "windows": "{ver}/win64/en-US/Firefox%20Setup%20{ver}.exe",
}
CHROME_STABLE_URL = "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions.json"
CHROMIUM_DEPS_URL = "https://chromium.googlesource.com/chromium/src/+/refs/tags/{version}/DEPS?format=TEXT"
BORINGSSL_REVISION = r"'boringssl_revision':\s*'([0-9a-f]{40})'"
CACHE = Path(os.environ.get("LEYLINE_CFT_CACHE", Path.home() / ".cache/leyline-cft"))
OUT = Path(os.environ.get("LEYLINE_ONESHOT_OUT", Path(tempfile.gettempdir()) / "leyline-oneshot"))
TODAY = date.today().isoformat()
CHROMIUM_BUILDS = {
    "chrome": {
        "product": "Google Chrome",
        "env": "LEYLINE_CHROME",
        "apt": {
            "base": "https://dl.google.com/linux/chrome/deb/",
            "index": "dists/stable/main/binary-amd64/Packages",
            "package": "google-chrome-stable",
            "binary": "opt/google/chrome/chrome",
        },
        "macos": {
            "url": "https://dl.google.com/chrome/mac/universal/stable/GGRO/googlechrome.dmg",
            "app": "Google Chrome.app",
            "exe": "Contents/MacOS/Google Chrome",
        },
        "windows": {
            "url": "https://dl.google.com/dl/chrome/install/googlechromestandaloneenterprise64.msi",
            "install": ["msiexec", "/i", "{installer}", "/qn", "/norestart"],
            "paths": [r"C:\Program Files\Google\Chrome\Application\chrome.exe"],
        },
    },
    "brave": {
        "product": "Brave Browser",
        "env": "LEYLINE_BRAVE",
        "release": {
            "url": "https://api.github.com/repos/brave/brave-browser/releases/latest",
            "name": r"Release v(?P<version>\d+\.\d+\.\d+) \(Chromium (?P<chromium>\d+)(?:\.\d+)*\)",
        },
        "apt": {
            "base": "https://brave-browser-apt-release.s3.brave.com/",
            "index": "dists/stable/main/binary-amd64/Packages",
            "package": "brave-browser",
            "binary": "opt/brave.com/brave/brave",
        },
        "macos": {
            "asset": "Brave-Browser-universal.dmg",
            "app": "Brave Browser.app",
            "exe": "Contents/MacOS/Brave Browser",
        },
        "windows": {
            "asset": "BraveBrowserStandaloneSetup.exe",
            "install": ["{installer}", "/silent", "/install"],
            "paths": [
                r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe",
                r"%LOCALAPPDATA%\BraveSoftware\Brave-Browser\Application\brave.exe",
            ],
        },
    },
}
OPERA = {
    "index": "https://get.geo.opera.com/pub/opera/desktop/",
    "version": r'href="(\d+\.\d+\.\d+\.\d+)/"',
    "deb": "{ver}/linux/opera-stable_{ver}_amd64.deb",
    "checksum": ".sha256sum",
    "binary": "usr/lib/x86_64-linux-gnu/opera-stable/opera",
    "brand": "Opera",
    "ua_chromium": r"Chrome/(\d+)\.",
    "ua_brand": r"OPR/(\d+)\.",
    "brand_version": "{major}.0.0.0",
    "docs": (
        ("crates/leyline/README.md", r"(The Opera overlay covers Chrome )\d+ to \d+", "{low} to {high}"),
        ("docs/guide/sessions.md", r"(Opera supports Chromium )\d+ to \d+", "{low} to {high}"),
        ("docs/guide/sessions.md", r"(pin an older Chrome such as\s+`Browser::Chrome)\d+", "{high}"),
    ),
}
GITHUB_TOKEN_ENV = "GITHUB_TOKEN"
SIGNERS = {
    "chrome": {"macos": "EQHXZ8M8AV", "windows": "Google LLC"},
    "brave": {"macos": "KL8N8XSYF4", "windows": "Brave Software, Inc."},
    "firefox": {"macos": "43AQ936H96", "windows": "Mozilla Corporation"},
}
SAFARI_APP = Path("/Applications/Safari.app")
SAFARIDRIVER = Path("/usr/bin/safaridriver")
SAFARIDRIVER_PORT = int(os.environ.get("LEYLINE_SAFARIDRIVER_PORT", "4444"))
H3_URL = os.environ.get("LEYLINE_H3_URL", "https://quic.browserleaks.com/?minify=1")
H3_RUNS = 2
CHROMIUM_H3_FLAGS = ("--enable-quic", "--origin-to-force-quic-on={host}:443")
PROFILES = ROOT / "crates/leyline/profiles"
BRANDS = PROFILES / "brands.toml"
CAPTURES = PROFILES / "captures"
QPACK_GOLDEN = ROOT / "crates/leyline/tests/data/h3_qpack.toml"
BROWSER_IDS = PROFILES / "browser_ids.toml"
TCP_SUFFIXES = ("", "-headful")
KEPT_IP_KEYS = frozenset({"dst_ip"})
OS_LABELS = {"macos": "macOS", "linux": "Linux", "windows": "Windows"}
HOST_OS = {"Darwin": "macos", "Linux": "linux", "Windows": "windows"}
FAMILIES = {
    "chrome": {"label": "Chrome", "name": "{label} {major}", "build_label": "{version}", "fill_gaps": False, "tcp_method": "--headless=new", "h3_method": "--headless=new", "h3_hosts": ("macos", "linux", "windows"), "ua_from_capture": False, "ua_marker": "Chrome/{major}."},
    "firefox": {"label": "Firefox", "name": "{label} {major}", "build_label": "{version}", "fill_gaps": True, "tcp_method": "--headless", "h3_method": "headful", "h3_hosts": ("linux",), "ua_from_capture": False, "ua_marker": "rv:{major}."},
    "brave": {"label": "Brave", "name": "{label} (Chromium {major})", "build_label": "{rest} (Chromium {major})", "fill_gaps": False, "tcp_method": "--headless=new", "h3_method": "--headless=new", "h3_hosts": ("macos", "linux", "windows"), "ua_from_capture": False, "ua_marker": "Chrome/{major}."},
    "safari": {"label": "Safari", "name": "{label} {major}", "build_label": "{version}", "fill_gaps": True, "tcp_method": "safaridriver", "h3_method": "headful", "h3_hosts": (), "ua_from_capture": True, "ua_marker": "Version/{major}."},
}
WAITS = {
    "page_load": 45.0,
    "page_poll": 0.5,
    "browser_close": 5.0,
    "browser_exit": 10.0,
    "driver_ready": 10.0,
    "driver_poll": 0.2,
    "webdriver_request": 60.0,
    "metadata_fetch": 30.0,
    "download": 300.0,
}
