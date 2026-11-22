#!/usr/bin/env python3
"""Render docs/assets/bench-light.svg and bench-dark.svg from pooled libs-table.py outputs."""

import argparse
import pathlib
import sys


THEMES = {
    "light": {
        "id": "bench-light",
        "bg": "#ece7dc",
        "border": "#1c1d1a",
        "border_op": "0.24",
        "divider": "#1c1d1a",
        "divider_op": "0.12",
        "ink": "#1c1d1a",
        "muted": "#5c5a54",
        "footer": "#7d796f",
        "accent_fill": "#5f7a2e",
        "accent_stroke": "#4c6422",
        "accent_text": "#4c6422",
        "peer_fill": "#5c5a54",
        "peer_fill_op": "0.13",
        "peer_stroke": "#5c5a54",
        "peer_stroke_op": "0.45",
    },
    "dark": {
        "id": "bench-dark",
        "bg": "#1c1d1a",
        "border": "#ece7dc",
        "border_op": "0.24",
        "divider": "#ece7dc",
        "divider_op": "0.14",
        "ink": "#ece7dc",
        "muted": "#a9a49a",
        "footer": "#8a867d",
        "accent_fill": "#c9daa5",
        "accent_stroke": "#c9daa5",
        "accent_text": "#c9daa5",
        "peer_fill": "#a9a49a",
        "peer_fill_op": "0.16",
        "peer_stroke": "#a9a49a",
        "peer_stroke_op": "0.48",
    },
}

SANS = "'IBM Plex Sans', -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif"
MONO = "'IBM Plex Mono', ui-monospace, 'SFMono-Regular', Menlo, monospace"


def parse_throughput(path: pathlib.Path, target_cell: str) -> dict[str, int]:
    cell = None
    out: dict[str, int] = {}
    for raw in path.read_text().splitlines():
        if not raw.strip():
            cell = None
            continue
        if raw.startswith("geometric-mean"):
            break
        parts = raw.split()
        if len(parts) >= 3 and parts[1] == "lib" and parts[2] == "req/s":
            cell = parts[0]
            continue
        if cell == target_cell and raw.startswith(" ") and len(parts) >= 2:
            out[parts[0]] = int(round(float(parts[1])))
    if not out:
        sys.exit(f"no {target_cell} rows found in {path}")
    return out


def parse_paced_cpu(path: pathlib.Path, target_cell: str) -> dict[str, float]:
    cell = None
    out: dict[str, float] = {}
    for raw in path.read_text().splitlines():
        if not raw.strip():
            cell = None
            continue
        if raw.startswith("open-loop paired"):
            break
        parts = raw.split()
        if len(parts) >= 3 and parts[1] == "lib" and parts[2] == "rate":
            cell = parts[0]
            continue
        if cell == target_cell and raw.startswith(" ") and len(parts) >= 7:
            out[parts[0]] = float(parts[6])
    if not out:
        sys.exit(f"no {target_cell} rows found in {path}")
    return out


def fmt_rate(rps: int) -> str:
    if rps >= 1_000_000:
        return f"{rps / 1_000_000:.2f}M"
    return f"{round(rps / 1_000)}k"


def bar_rows(
    items: list[tuple[str, float, str, bool]],
    *,
    label_x: int,
    bar_x: int,
    y0: int,
    step: int,
    height: int,
    text_dy: float,
    max_width: int,
    theme: dict[str, str],
    comment_fmt,
) -> list[str]:
    hi = max(val for _, val, _, _ in items)
    lines: list[str] = []
    for idx, (label, val, text, highlight) in enumerate(items):
        y = y0 + idx * step
        ty = y + text_dy
        w = max(1, round(max_width * val / hi))
        vx = bar_x + w + 8
        if comment_fmt is not None:
            lines.append(f"  <!-- {label}: {comment_fmt(val)} -->")
        if highlight:
            lines.append(
                f'  <text x="{label_x}" y="{ty:g}" fill="{theme["ink"]}" font-family="{SANS}" font-size="12.5" font-weight="600" text-anchor="end">{label}</text>'
            )
            lines.append(
                f'  <rect x="{bar_x}" y="{y}" width="{w}" height="{height}" rx="4" fill="{theme["accent_fill"]}" fill-opacity="0.16" stroke="{theme["accent_stroke"]}" stroke-width="1.5"/>'
            )
            lines.append(
                f'  <text x="{vx}" y="{ty:g}" fill="{theme["accent_text"]}" font-family="{MONO}" font-size="11.5" font-weight="600">{text}</text>'
            )
        else:
            lines.append(
                f'  <text x="{label_x}" y="{ty:g}" fill="{theme["muted"]}" font-family="{SANS}" font-size="12" font-weight="500" text-anchor="end">{label}</text>'
            )
            lines.append(
                f'  <rect x="{bar_x}" y="{y}" width="{w}" height="{height}" rx="4" fill="{theme["peer_fill"]}" fill-opacity="{theme["peer_fill_op"]}" stroke="{theme["peer_stroke"]}" stroke-opacity="{theme["peer_stroke_op"]}" stroke-width="1"/>'
            )
            lines.append(
                f'  <text x="{vx}" y="{ty:g}" fill="{theme["ink"]}" font-family="{MONO}" font-size="11">{text}</text>'
            )
        if idx + 1 < len(items):
            lines.append("")
    return lines


def render_svg(
    theme: dict[str, str],
    h2: dict[str, int],
    browser: dict[str, int],
    cpu: dict[str, float],
    h1: dict[str, int],
) -> str:
    left_items = sorted(
        [
            ("Leyline", h2["leyline"], fmt_rate(h2["leyline"]), True),
            ("hyper-util", h2["hyper"], fmt_rate(h2["hyper"]), False),
            ("reqwest", h2["reqwest"], fmt_rate(h2["reqwest"]), False),
            ("libcurl", h2["curl"], fmt_rate(h2["curl"]), False),
            ("Go net/http", h2["nethttp"], fmt_rate(h2["nethttp"]), False),
        ],
        key=lambda item: -item[1],
    )
    browser_items = sorted(
        [
            ("Leyline", browser["leyline"], fmt_rate(browser["leyline"]), True),
            ("wreq", browser["wreq"], fmt_rate(browser["wreq"]), False),
            ("tls-client", browser["tlsclient"], fmt_rate(browser["tlsclient"]), False),
        ],
        key=lambda item: -item[1],
    )
    cpu_items = sorted(
        [
            ("Leyline", cpu["leyline"], f'{cpu["leyline"]:.2f} µs', True),
            ("hyper-util", cpu["hyper"], f'{cpu["hyper"]:.2f} µs', False),
            ("reqwest", cpu["reqwest"], f'{cpu["reqwest"]:.2f} µs', False),
        ],
        key=lambda item: item[1],
    )
    left_desc = ", ".join(
        f"{name} {text}" + (" req/s" if idx == 0 else "")
        for idx, (name, _, text, _) in enumerate(left_items)
    )
    desc = (
        f"Left panel compares HTTP/2 throughput across general clients "
        f"({left_desc}). "
        f"Right panel compares Chrome 146 browser-profile throughput "
        f"(Leyline {fmt_rate(browser['leyline'])} req/s vs wreq {fmt_rate(browser['wreq'])} "
        f"and tls-client {fmt_rate(browser['tlsclient'])}) and open-loop client CPU per request "
        f"at 50k req/s (Leyline {cpu['leyline']:.2f} µs vs hyper-util {cpu['hyper']:.2f} µs "
        f"and reqwest {cpu['reqwest']:.2f} µs)."
    )
    footer = (
        f"run_per_core reaches {fmt_rate(h2['leyline-cores'])} HTTP/2 req/s "
        f"and {fmt_rate(h1['leyline-cores'])} HTTP/1.1 req/s"
    )
    tid = theme["id"]
    lines = [
        f'<svg xmlns="http://www.w3.org/2000/svg" role="img" aria-labelledby="{tid}-title {tid}-desc" viewBox="0 0 920 316" width="920" height="316">',
        f'  <title id="{tid}-title">Leyline HTTP/2 throughput, browser profile speed, and client CPU per request</title>',
        f'  <desc id="{tid}-desc">{desc}</desc>',
        "",
        "  <!-- Card Background -->",
        f'  <rect x="0.5" y="0.5" width="919" height="315" rx="8" fill="{theme["bg"]}" stroke="{theme["border"]}" stroke-opacity="{theme["border_op"]}"/>',
        "",
        "  <!-- Left Panel: General HTTP/2 -->",
        f'  <text x="24" y="32" fill="{theme["ink"]}" font-family="{SANS}" font-size="13.5" font-weight="600">General HTTP/2 throughput (8×256)</text>',
        f'  <text x="444" y="32" fill="{theme["muted"]}" font-family="{MONO}" font-size="10.5" text-anchor="end">req/s · higher is better</text>',
        f'  <line x1="24" y1="44" x2="444" y2="44" stroke="{theme["divider"]}" stroke-opacity="{theme["divider_op"]}" stroke-width="1"/>',
        "",
        *bar_rows(
            left_items,
            label_x=106,
            bar_x=116,
            y0=64,
            step=38,
            height=26,
            text_dy=17.5,
            max_width=260,
            theme=theme,
            comment_fmt=lambda v: f"{int(v):,}",
        ),
        "",
        "  <!-- Center Divider -->",
        f'  <line x1="468" y1="20" x2="468" y2="296" stroke="{theme["divider"]}" stroke-opacity="{theme["divider_op"]}" stroke-width="1"/>',
        "",
        "  <!-- Right Panel Top: Browser Profiles -->",
        f'  <text x="492" y="32" fill="{theme["ink"]}" font-family="{SANS}" font-size="13.5" font-weight="600">With Chrome TLS + HTTP/2 (8×256)</text>',
        f'  <text x="896" y="32" fill="{theme["muted"]}" font-family="{MONO}" font-size="10.5" text-anchor="end">req/s · higher is better</text>',
        f'  <line x1="492" y1="44" x2="896" y2="44" stroke="{theme["divider"]}" stroke-opacity="{theme["divider_op"]}" stroke-width="1"/>',
        "",
        *bar_rows(
            browser_items,
            label_x=566,
            bar_x=576,
            y0=64,
            step=36,
            height=26,
            text_dy=17.5,
            max_width=240,
            theme=theme,
            comment_fmt=None,
        ),
        "",
        "  <!-- Right Panel Bottom: Client CPU / req at 50k req/s -->",
        f'  <text x="492" y="194" fill="{theme["ink"]}" font-family="{SANS}" font-size="13.5" font-weight="600">Client CPU per request at 50k req/s</text>',
        f'  <text x="896" y="194" fill="{theme["muted"]}" font-family="{MONO}" font-size="10.5" text-anchor="end">µs · lower is better</text>',
        f'  <line x1="492" y1="205" x2="896" y2="205" stroke="{theme["divider"]}" stroke-opacity="{theme["divider_op"]}" stroke-width="1"/>',
        "",
        *bar_rows(
            cpu_items,
            label_x=566,
            bar_x=576,
            y0=216,
            step=28,
            height=22,
            text_dy=15,
            max_width=226,
            theme=theme,
            comment_fmt=None,
        ),
        "",
        "  <!-- Left Footer Note -->",
        f'  <text x="24" y="287" fill="{theme["footer"]}" font-family="{MONO}" font-size="10.5">{footer}</text>',
        "</svg>",
    ]
    return "\n".join(lines) + "\n"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--h2", type=pathlib.Path, required=True, help="Pooled general HTTP/2 table")
    parser.add_argument("--browser", type=pathlib.Path, required=True, help="Pooled Chrome 146 HTTP/2 table")
    parser.add_argument("--paced", type=pathlib.Path, required=True, help="Pooled open-loop HTTP/2 table")
    parser.add_argument("--h1", type=pathlib.Path, required=True, help="Pooled general HTTP/1.1 table")
    parser.add_argument("--out-dir", type=pathlib.Path, default=pathlib.Path("docs/assets"))
    args = parser.parse_args()

    h2 = parse_throughput(args.h2, "8c256")
    browser = parse_throughput(args.browser, "8c256")
    cpu = parse_paced_cpu(args.paced, "c8x256")
    h1 = parse_throughput(args.h1, "64par")

    args.out_dir.mkdir(parents=True, exist_ok=True)
    for name, theme in THEMES.items():
        target = args.out_dir / f"bench-{name}.svg"
        target.write_text(render_svg(theme, h2, browser, cpu, h1))
        print(f"wrote {target}")


if __name__ == "__main__":
    main()
