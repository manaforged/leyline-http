from __future__ import annotations

import platform as plat
import subprocess

from .binaries import major_of
from .chrome import fill_chromium, live_brave_major, live_chrome_major, live_gap
from .opera import live_opera_version, opera_has
from .firefox import live_firefox_version
from .land import bundled_majors, missing_majors
from .safari import safari_host_version

Row = tuple[str, str, str, str, str]


def bundled_text(majors: list[int]) -> str:
    return ",".join(str(m) for m in majors) or "none"


def version_row(family: str, live: str, missing: list[int]) -> Row:
    note = f"missing {missing}" if missing else "current"
    return (family, bundled_text(bundled_majors(family)), live, "fill" if missing else "ok", note)


def chrome_row() -> Row:
    major, version = live_chrome_major()
    return version_row("chrome", version, missing_majors("chrome", major))


def firefox_row() -> Row:
    version = live_firefox_version()
    return version_row("firefox", version, missing_majors("firefox", int(version.split(".", 1)[0])))


def safari_row() -> Row:
    host = safari_host_version()
    major = int(host.split(".", 1)[0]) if host[:1].isdigit() else 0
    have = bundled_majors("safari")
    gap = [] if major in have else [major]
    note = f"Safari.app capture {gap}" if gap else "current"
    return ("safari", bundled_text(have), host, "fill" if gap else "ok", note)


def edge_row(chrome_behind: bool) -> Row:
    chrome_have = bundled_majors("chrome")
    newest = chrome_have[-1] if chrome_have else 0
    edge_major, edge_version = live_edge_major()
    gap = edge_major > newest
    note = f"needs Chrome {edge_major}" if gap else "ChromiumBrand::Edge on current Chrome TLS"
    return ("edge", f"overlay Chrome{newest}", edge_version, "fill" if chrome_behind or gap else "ok", note)


def brave_row() -> Row:
    major, name = live_brave_major()
    return version_row("brave", name, live_gap("brave", major))


def opera_row() -> Row:
    version = live_opera_version()
    gap = not opera_has(major_of(version))
    return ("opera", "brands.toml", version, "fill" if gap else "ok", "brand row missing" if gap else "current")


def plan() -> dict[str, list[dict[str, object]]]:
    firefox = major_of(live_firefox_version())
    majors = [("chrome", major) for major in live_gap("chrome", live_chrome_major()[0])]
    majors += [("brave", major) for major in live_gap("brave", live_brave_major()[0])]
    majors += [("firefox", major) for major in missing_majors("firefox", firefox)]
    opera = major_of(live_opera_version())
    return {
        "capture": [{"family": family, "major": major} for family, major in majors],
        "brand": [] if opera_has(opera) else [{"family": "opera", "major": opera}],
    }


def catalog() -> list[Row]:
    rows = [chrome_row(), firefox_row(), brave_row(), opera_row()]
    if plat.system() == "Darwin":
        rows.append(safari_row())
    rows.append(edge_row(rows[0][3] == "fill"))
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


def print_catalog(rows: list[tuple[str, str, str, str, str]]) -> None:
    print(f"{'family':<12} {'bundled':<28} {'live':<22} {'do':<10} note")
    print("-" * 110)
    for fam, bundled, live, action, note in rows:
        print(f"{fam:<12} {bundled:<28} {live:<22} {action:<10} {note}")
    print("suite: " + ", ".join(row[0] for row in rows))


def fill_edge(dry: bool, pending_chrome: set[int]) -> list[tuple[str, int, str]]:
    chrome_have = bundled_majors("chrome")
    newest = max(chrome_have + list(pending_chrome), default=0)
    edge_maj, edge_ver = live_edge_major()
    if edge_maj > newest:
        print(f"edge: Chrome {newest} behind Edge/Chromium {edge_ver}; fill chrome first")
        return fill_chromium("chrome", dry)
    print(f"edge: overlay on Chrome {newest} (TLS/H2 Chrome, HTTP Edg/{newest})")
    return []
