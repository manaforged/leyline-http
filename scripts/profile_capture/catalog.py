from __future__ import annotations

import platform as plat
import subprocess

from .chrome import fill_chrome, live_chrome_major
from .firefox import live_firefox_version
from .land import bundled_majors, missing_majors
from .safari import safari_host_version

def catalog() -> list[tuple[str, str, str, str, str]]:
    """family, bundled, live, action (fill/skip/ok), note"""
    rows: list[tuple[str, str, str, str, str]] = []
    chrome_have = bundled_majors("chrome")
    cmaj, cver = live_chrome_major()
    chrome_miss = missing_majors("chrome", cmaj)
    rows.append((
        "chrome",
        ",".join(str(m) for m in chrome_have) or "none",
        cver,
        "fill" if chrome_miss else "ok",
        f"missing {chrome_miss}" if chrome_miss else "current",
    ))
    ff_have = bundled_majors("firefox")
    ff_ver = live_firefox_version()
    ff_maj = int(ff_ver.split(".", 1)[0])
    firefox_miss = missing_majors("firefox", ff_maj)
    rows.append((
        "firefox",
        ",".join(str(m) for m in ff_have) or "none",
        ff_ver,
        "fill" if firefox_miss else "ok",
        f"missing {firefox_miss}" if firefox_miss else "current",
    ))
    if plat.system() == "Darwin":
        safari_host = safari_host_version()
        safari_maj = int(safari_host.split(".", 1)[0]) if safari_host[:1].isdigit() else 0
        safari_have = bundled_majors("safari")
        safari_gap = [] if safari_maj in safari_have else [safari_maj]
        rows.append((
            "safari",
            ",".join(str(m) for m in safari_have) or "none",
            safari_host,
            "fill" if safari_gap else "ok",
            f"Safari.app capture {safari_gap}" if safari_gap else "current",
        ))
    chrome_newest = chrome_have[-1] if chrome_have else 0
    edge_maj, edge_ver = live_edge_major()
    edge_gap = edge_maj > chrome_newest
    rows.append((
        "edge",
        f"overlay Chrome{chrome_newest}",
        edge_ver,
        "fill" if chrome_miss or edge_gap else "ok",
        "ChromiumBrand::Edge on current Chrome TLS" if not edge_gap
        else f"needs Chrome {edge_maj}",
    ))
    return rows


def live_edge_major() -> tuple[int, str]:
    try:
        ver = subprocess.check_output(
            ["defaults", "read", "/Applications/Microsoft Edge.app/Contents/Info",
             "CFBundleShortVersionString"],
            text=True,
            stderr=subprocess.DEVNULL,
        ).strip()
        return int(ver.split(".", 1)[0]), ver
    except (subprocess.CalledProcessError, ValueError, FileNotFoundError):
        cmaj, cver = live_chrome_major()
        return cmaj, f"Chromium {cver} lockstep"


def sudo_ok() -> bool:
    return subprocess.run(["sudo", "-n", "true"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0


def print_catalog(rows: list[tuple[str, str, str, str, str]]) -> None:
    print(f"{'family':<12} {'bundled':<28} {'live':<22} {'do':<10} note")
    print("-" * 110)
    for fam, bundled, live, action, note in rows:
        print(f"{fam:<12} {bundled:<28} {live:<22} {action:<10} {note}")
    print("suite: " + ", ".join(row[0] for row in rows))


def fill_edge(dry: bool) -> list[tuple[str, int, str]]:
    chrome_have = bundled_majors("chrome")
    newest = chrome_have[-1] if chrome_have else 0
    edge_maj, edge_ver = live_edge_major()
    if edge_maj > newest:
        print(f"edge: Chrome {newest} behind Edge/Chromium {edge_ver}; fill chrome first")
        return fill_chrome(dry)
    print(f"edge: overlay on Chrome {newest} (TLS/H2 Chrome, HTTP Edg/{newest})")
    return []
