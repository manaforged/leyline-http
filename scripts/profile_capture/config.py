from __future__ import annotations

import os
import tempfile
from datetime import date
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
PEET_URL = os.environ.get("PEET_URL", "https://tls.peet.ws/api/all")
FF_URL = "https://product-details.mozilla.org/1.0/firefox_versions.json"
FF_RELEASES_URL = "https://product-details.mozilla.org/1.0/firefox.json"
CACHE = Path(os.environ.get("LEYLINE_CFT_CACHE", Path.home() / ".cache/leyline-cft"))
OUT = Path(os.environ.get("LEYLINE_ONESHOT_OUT", Path(tempfile.gettempdir()) / "leyline-oneshot"))
TODAY = date.today().isoformat()
CHROME_DEB_BASE = "https://dl.google.com/linux/chrome/deb/"
CHROME_DEB_INDEX = CHROME_DEB_BASE + "dists/stable/main/binary-amd64/Packages"
MAC_CHROME = Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
SAFARI_APP = Path("/Applications/Safari.app")
SAFARIDRIVER = Path("/usr/bin/safaridriver")
SAFARIDRIVER_PORT = int(os.environ.get("LEYLINE_SAFARIDRIVER_PORT", "4444"))
H3_URL = os.environ.get("LEYLINE_H3_URL", "https://quic.browserleaks.com/?minify=1")
H3_RUNS = 2
PROFILES = ROOT / "crates/leyline/profiles"
CAPTURES = PROFILES / "captures"
QPACK_GOLDEN = ROOT / "crates/leyline/tests/data/h3_qpack.toml"
KEPT_IP_KEYS = frozenset({"dst_ip"})
OS_LABELS = {"macos": "macOS", "linux": "Linux", "windows": "Windows"}
HOST_OS = {"Darwin": "macos", "Linux": "linux", "Windows": "windows"}
FAMILIES = {
    "chrome": {"label": "Chrome", "tcp_method": "--headless=new", "h3": False, "ua_from_capture": False},
    "firefox": {"label": "Firefox", "tcp_method": "--headless", "h3": True, "ua_from_capture": False},
    "safari": {"label": "Safari", "tcp_method": "safaridriver", "h3": False, "ua_from_capture": True},
}
