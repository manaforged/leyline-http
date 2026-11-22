#!/usr/bin/env python3
"""Render docs/assets/bench-light.svg and bench-dark.svg from pooled libs-table.py outputs."""

import argparse
import collections
import math
import pathlib
import statistics
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


def parse_throughput_full(path: pathlib.Path, target_cell: str) -> dict[str, tuple[int, float]]:
    cell = None
    out: dict[str, tuple[int, float]] = {}
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
        if cell == target_cell and raw.startswith(" ") and len(parts) >= 6:
            out[parts[0]] = (int(round(float(parts[1]))), float(parts[5]))
    if not out:
        sys.exit(f"no {target_cell} rows found in {path}")
    return out


def parse_throughput(path: pathlib.Path, target_cell: str) -> dict[str, int]:
    return {k: v[0] for k, v in parse_throughput_full(path, target_cell).items()}


def parse_paced_full(path: pathlib.Path, target_cell: str) -> dict[str, tuple[int, float, float]]:
    cell = None
    out: dict[str, tuple[int, float, float]] = {}
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
        if cell == target_cell and raw.startswith(" ") and len(parts) >= 8:
            out[parts[0]] = (int(round(float(parts[4]))), float(parts[6]), float(parts[7]))
    if not out:
        sys.exit(f"no {target_cell} rows found in {path}")
    return out


def parse_paced_cpu(path: pathlib.Path, target_cell: str) -> dict[str, float]:
    return {k: v[1] for k, v in parse_paced_full(path, target_cell).items()}


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


def parse_paired_ratios(path: pathlib.Path, target_cell: str) -> dict[str, tuple[float, float]]:
    parent = path.parent
    stem = parent.name
    sibling_rows: list[pathlib.Path] = []
    if stem.endswith("-r1"):
        prefix = stem[:-1]
        sibling_rows = sorted(
            p / "rows.txt"
            for p in parent.parent.glob(f"{prefix}[0-9]*")
            if (p / "rows.txt").is_file()
        )
    if sibling_rows:
        runs: list[dict[tuple[str, str], dict[str, tuple[float, float]]]] = []
        for row_file in sibling_rows:
            run: dict[tuple[str, str], dict[str, tuple[float, float]]] = collections.defaultdict(dict)
            for line in row_file.read_text().splitlines():
                p = line.split()
                if len(p) != 11 or p[1] != target_cell:
                    continue
                dn = int(p[7]) - int(p[3])
                rps = dn * 1e9 / (int(p[10]) - int(p[6]))
                cpu = (float(p[9]) - float(p[5])) * 1000.0 / dn
                run[(p[1], p[2])][p[0]] = (rps, cpu)
            if run:
                runs.append(run)
        peers = {lib for run in runs for _, lib in run if lib != "leyline"}
        out: dict[str, tuple[float, float]] = {}
        for peer in peers:
            rps_means = []
            cpu_means = []
            for run in runs:
                ours = run.get((target_cell, "leyline"), {})
                theirs = run.get((target_cell, peer), {})
                shared = [r for r in ours if r in theirs]
                if not shared:
                    continue
                rps_means.append(statistics.mean(math.log(ours[r][0] / theirs[r][0]) for r in shared))
                cpu_means.append(statistics.mean(math.log(ours[r][1] / theirs[r][1]) for r in shared))
            if rps_means:
                out[peer] = (math.exp(statistics.mean(rps_means)), math.exp(statistics.mean(cpu_means)))
        if out:
            return out

    out = {}
    in_ratios = False
    for raw in path.read_text().splitlines():
        if raw.startswith("geometric-mean paired ratio"):
            in_ratios = True
            continue
        if not in_ratios:
            continue
        parts = raw.split()
        if len(parts) < 12 or parts[0] != target_cell or parts[1] != "leyline" or parts[2] != "vs":
            continue
        peer = parts[3]
        try:
            rps_i = parts.index("req/s")
            cpu_i = parts.index("CPU/req")
            out[peer] = (float(parts[rps_i + 1]), float(parts[cpu_i + 1]))
        except (ValueError, IndexError):
            continue
    return out


def render_social_html(
    version: str,
    h2_full: dict[str, tuple[int, float]],
    h2_ratios: dict[str, tuple[float, float]],
    browser_full: dict[str, tuple[int, float]],
    browser_ratios: dict[str, tuple[float, float]],
    paced_full: dict[str, tuple[int, float, float]],
    h1_full: dict[str, tuple[int, float]],
) -> str:
    h2_ley_rps, h2_ley_cpu = h2_full["leyline"]
    h2_hyper_cpu_ratio = (
        h2_ratios["hyper"][1]
        if "hyper" in h2_ratios
        else h2_ley_cpu / h2_full["hyper"][1]
    )
    h2_rows_spec = [
        ("leyline", "Leyline", True),
        ("h2", "bare h2 crate", False),
        ("hyper", "hyper-util", False),
        ("curl", "libcurl", False),
        ("reqwest", "reqwest", False),
        ("nethttp", "Go net/http", False),
    ]
    h2_rows_spec.sort(key=lambda r: -h2_full[r[0]][0])
    h2_max = h2_full[h2_rows_spec[0][0]][0]

    def bar_markup(pct: float, label_text: str) -> str:
        if pct < 40.0:
            return (
                f'<div class="bar-wrap">'
                f'<div class="bar-fill" style="width:{pct:.1f}%"></div>'
                f'<span class="bar-val outside" style="left:calc({pct:.1f}% + 9px)">{label_text}</span>'
                f"</div>"
            )
        return (
            f'<div class="bar-wrap">'
            f'<div class="bar-fill" style="width:{pct:.1f}%"></div>'
            f'<span class="bar-val">{label_text}</span>'
            f"</div>"
        )

    tr_h2 = []
    for key, label, hl in h2_rows_spec:
        rps, cpu_us = h2_full[key]
        pct = max(4.0, 100.0 * rps / h2_max)
        p99 = f"{paced_full[key][0]} µs" if key in paced_full else "—"
        if hl:
            lead = "baseline"
        else:
            rps_r, cpu_r = h2_ratios.get(key, (h2_ley_rps / rps, h2_ley_cpu / cpu_us))
            lead = f"{rps_r:.2f}× · {cpu_r:.2f}× CPU"
        cls = ' class="focal"' if hl else ""
        tr_h2.append(
            f"""          <tr{cls}>
            <td class="name">{label}</td>
            <td class="bar-cell">{bar_markup(pct, fmt_rate(rps))}</td>
            <td class="num">{cpu_us:.2f} µs</td>
            <td class="num">{p99}</td>
            <td class="lead">{lead}</td>
          </tr>"""
        )

    br_ley_rps = browser_full["leyline"][0]
    br_rows_spec = [
        ("leyline", "Leyline", True),
        ("wreq", "wreq", False),
        ("tlsclient", "tls-client", False),
    ]
    br_rows_spec.sort(key=lambda r: -browser_full[r[0]][0])
    br_max = browser_full[br_rows_spec[0][0]][0]
    tr_br = []
    for key, label, hl in br_rows_spec:
        rps, cpu_us = browser_full[key]
        pct = max(6.0, 100.0 * rps / br_max)
        if hl:
            ratio = "baseline"
        else:
            rps_r = browser_ratios.get(key, (br_ley_rps / rps, 1.0))[0]
            ratio = f"{rps_r:.2f}×"
        cls = ' class="focal"' if hl else ""
        tr_br.append(
            f"""          <tr{cls}>
            <td class="name">{label}</td>
            <td class="bar-cell">{bar_markup(pct, fmt_rate(rps))}</td>
            <td class="num">{cpu_us:.2f} µs</td>
            <td class="lead">{ratio}</td>
          </tr>"""
        )

    ley_p99, ley_pcpu, ley_rss = paced_full["leyline"]
    h1_cores_rps = h1_full["leyline-cores"][0]

    return f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>leyline-http {version} benchmarks</title>
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=Newsreader:opsz,wght@6..72,500&family=IBM+Plex+Sans:wght@400;500;600&family=IBM+Plex+Mono:wght@400;500;600&display=swap" rel="stylesheet">
<style>
  :root {{
    --paper: #1c1d1a;
    --paper-2: #23241f;
    --ink: #ece7dc;
    --muted: #a9a49a;
    --soft: #8a867d;
    --rule: rgba(236, 231, 220, 0.14);
    --rule-solid: rgba(236, 231, 220, 0.28);
    --accent: #c9daa5;
    --accent-tint: rgba(201, 218, 165, 0.13);
    --bar-peer: rgba(169, 164, 154, 0.18);
    --bar-peer-border: rgba(169, 164, 154, 0.38);
  }}
  * {{ box-sizing: border-box; margin: 0; padding: 0; }}
  html, body {{
    width: 1200px;
    height: 675px;
    overflow: hidden;
    background: var(--paper);
    color: var(--ink);
    font-family: 'IBM Plex Sans', -apple-system, BlinkMacSystemFont, sans-serif;
    -webkit-font-smoothing: antialiased;
  }}
  .card {{
    width: 1200px;
    height: 675px;
    padding: 36px 44px 28px;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
  }}
  header {{
    display: flex;
    justify-content: space-between;
    align-items: flex-end;
    padding-bottom: 20px;
    border-bottom: 1px solid var(--rule);
  }}
  .brand-row {{
    display: flex;
    align-items: baseline;
    gap: 14px;
  }}
  h1 {{
    font-family: 'Newsreader', Georgia, serif;
    font-weight: 500;
    font-size: 40px;
    line-height: 1.05;
    letter-spacing: -0.015em;
    color: var(--ink);
  }}
  .ver {{
    font-family: 'IBM Plex Mono', monospace;
    font-size: 14px;
    font-weight: 600;
    color: var(--accent);
    background: var(--accent-tint);
    border: 1px solid rgba(201, 218, 165, 0.45);
    border-radius: 4px;
    padding: 3px 9px;
    letter-spacing: 0.03em;
  }}
  .sub {{
    margin-top: 8px;
    font-family: 'IBM Plex Mono', monospace;
    font-size: 12.5px;
    color: var(--muted);
    letter-spacing: 0.01em;
  }}
  .hero-stat {{
    text-align: right;
  }}
  .hero-num {{
    font-family: 'IBM Plex Mono', monospace;
    font-size: 40px;
    font-weight: 600;
    line-height: 1;
    color: var(--accent);
    letter-spacing: -0.02em;
  }}
  .hero-num span {{
    font-size: 18px;
    font-weight: 500;
    color: var(--ink);
    margin-left: 4px;
  }}
  .hero-cap {{
    margin-top: 6px;
    font-family: 'IBM Plex Mono', monospace;
    font-size: 12.5px;
    color: var(--muted);
  }}
  main {{
    flex: 1;
    display: grid;
    grid-template-columns: 1.48fr 1fr;
    gap: 36px;
    padding: 24px 0 20px;
    min-height: 0;
  }}
  .col-right {{
    border-left: 1px solid var(--rule);
    padding-left: 36px;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
  }}
  .sec-head {{
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    margin-bottom: 14px;
  }}
  .sec-title {{
    font-size: 16px;
    font-weight: 600;
    color: var(--ink);
    letter-spacing: -0.005em;
  }}
  .sec-meta {{
    font-family: 'IBM Plex Mono', monospace;
    font-size: 11.5px;
    color: var(--soft);
  }}
  table {{
    width: 100%;
    border-collapse: collapse;
    font-variant-numeric: tabular-nums;
  }}
  th {{
    font-family: 'IBM Plex Mono', monospace;
    font-size: 11px;
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--soft);
    text-align: right;
    padding: 0 8px 10px;
    border-bottom: 1px solid var(--rule);
  }}
  th:first-child {{ text-align: left; padding-left: 12px; }}
  th.bar-th {{ text-align: left; padding-left: 10px; }}
  .col-left td {{
    padding: 15px 8px;
  }}
  .col-right td {{
    padding: 13px 8px;
  }}
  td {{
    border-bottom: 1px solid rgba(236, 231, 220, 0.07);
    vertical-align: middle;
  }}
  tr:last-child td {{ border-bottom: none; }}
  td.name {{
    font-size: 15.5px;
    font-weight: 500;
    color: var(--muted);
    padding-left: 12px;
    white-space: nowrap;
  }}
  td.num {{
    font-family: 'IBM Plex Mono', monospace;
    font-size: 14.5px;
    color: var(--ink);
    text-align: right;
    white-space: nowrap;
  }}
  td.lead {{
    font-family: 'IBM Plex Mono', monospace;
    font-size: 13px;
    color: var(--muted);
    text-align: right;
    white-space: nowrap;
    padding-right: 10px;
  }}
  td.bar-cell {{
    width: 196px;
    padding: 8px 10px !important;
  }}
  .col-right td.bar-cell {{
    width: 142px;
  }}
  .bar-wrap {{
    position: relative;
    height: 30px;
    display: flex;
    align-items: center;
  }}
  .bar-fill {{
    height: 30px;
    border-radius: 4px;
    background: var(--bar-peer);
    border: 1px solid var(--bar-peer-border);
  }}
  .bar-val {{
    position: absolute;
    left: 10px;
    font-family: 'IBM Plex Mono', monospace;
    font-size: 14px;
    font-weight: 600;
    color: var(--ink);
    white-space: nowrap;
  }}
  tr.focal td {{
    background: var(--accent-tint);
  }}
  tr.focal td:first-child {{
    border-left: 3px solid var(--accent);
    padding-left: 9px;
    color: var(--ink);
    font-weight: 600;
  }}
  tr.focal .bar-fill {{
    background: rgba(201, 218, 165, 0.22);
    border: 1.5px solid var(--accent);
  }}
  tr.focal .bar-val,
  tr.focal td.num,
  tr.focal td.lead {{
    color: var(--accent);
    font-weight: 600;
  }}
  .highlights {{
    border-top: 1px solid var(--rule);
    padding-top: 20px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }}
  .hl-row {{
    display: grid;
    grid-template-columns: 118px 1fr;
    gap: 12px;
    align-items: baseline;
  }}
  .hl-k {{
    font-family: 'IBM Plex Mono', monospace;
    font-size: 15px;
    font-weight: 600;
    color: var(--accent);
    white-space: nowrap;
  }}
  .hl-v {{
    font-size: 13.5px;
    color: var(--muted);
    line-height: 1.38;
  }}
  .hl-v strong {{
    color: var(--ink);
    font-weight: 500;
  }}
  footer {{
    border-top: 1px solid var(--rule);
    padding-top: 14px;
    display: flex;
    justify-content: space-between;
    font-family: 'IBM Plex Mono', monospace;
    font-size: 12px;
    color: var(--soft);
  }}
  footer strong {{
    color: var(--muted);
    font-weight: 500;
  }}
</style>
</head>
<body>
<div class="card">
  <header>
    <div>
      <div class="brand-row">
        <h1>leyline-http</h1>
        <span class="ver">{version}</span>
      </div>
      <div class="sub">Rust HTTP/1.1 · HTTP/2 · HTTP/3 · BoringSSL · Verified TLS 1.3 · Ryzen 9 9950X3D · 3-run medians</div>
    </div>
    <div class="hero-stat">
      <div class="hero-num">{fmt_rate(h2_ley_rps)}<span>req/s</span></div>
      <div class="hero-cap">HTTP/2 8×256 · {h2_hyper_cpu_ratio:.2f}× hyper-util CPU</div>
    </div>
  </header>

  <main>
    <section class="col-left">
      <div class="sec-head">
        <div class="sec-title">General HTTP/2 (8 connections · 256 in flight)</div>
        <div class="sec-meta">identical headers · 30s timeout</div>
      </div>
      <table>
        <thead>
          <tr>
            <th>Client</th>
            <th class="bar-th">req/s</th>
            <th>CPU/req</th>
            <th>50k p99</th>
            <th>Leyline lead</th>
          </tr>
        </thead>
        <tbody>
{chr(10).join(tr_h2)}
        </tbody>
      </table>
    </section>

    <div class="col-right">
      <section>
        <div class="sec-head">
          <div class="sec-title">Chrome 146 TLS + HTTP/2</div>
          <div class="sec-meta">8×256 · req/s</div>
        </div>
        <table>
          <thead>
            <tr>
              <th>Client</th>
              <th class="bar-th">req/s</th>
              <th>CPU</th>
              <th>Lead</th>
            </tr>
          </thead>
          <tbody>
{chr(10).join(tr_br)}
          </tbody>
        </table>
      </section>

      <section class="highlights">
        <div class="hl-row">
          <div class="hl-k">{fmt_rate(h1_cores_rps)} H1</div>
          <div class="hl-v"><strong>run_per_core</strong> leads fasthttp at 8 &amp; 64 workers (0.74×&nbsp;CPU)</div>
        </div>
        <div class="hl-row">
          <div class="hl-k">{ley_pcpu:.2f} µs CPU</div>
          <div class="hl-v">Open-loop 50k H2 · <strong>{ley_p99}&nbsp;µs</strong> p99 · <strong>{ley_rss:.1f}&nbsp;MiB</strong>&nbsp;RSS</div>
        </div>
        <div class="hl-row">
          <div class="hl-k">DoH + ECH</div>
          <div class="hl-v">RFC 8484 DoH, TLS 1.3 ECH + retry, PKCS#12, Unix sockets, <strong>28&nbsp;Kani&nbsp;proofs</strong></div>
        </div>
      </section>
    </div>
  </main>

  <footer>
    <div><strong>github.com/manaforged/leyline-http</strong> · crates.io/crates/leyline-http</div>
    <div>Full tables, 95% CIs &amp; raw receipts in BENCHMARKS.md · MIT</div>
  </footer>
</div>
</body>
</html>
"""


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--h2", type=pathlib.Path, required=True, help="Pooled general HTTP/2 table")
    parser.add_argument("--browser", type=pathlib.Path, required=True, help="Pooled Chrome 146 HTTP/2 table")
    parser.add_argument("--paced", type=pathlib.Path, required=True, help="Pooled open-loop HTTP/2 table")
    parser.add_argument("--h1", type=pathlib.Path, required=True, help="Pooled general HTTP/1.1 table")
    parser.add_argument("--out-dir", type=pathlib.Path, default=pathlib.Path("docs/assets"))
    parser.add_argument("--social-html", type=pathlib.Path, help="Optional 1200x675 social benchmark card HTML")
    parser.add_argument("--version", default="v0.3.0", help="Version badge on the social card")
    args = parser.parse_args()

    h2_full = parse_throughput_full(args.h2, "8c256")
    h2_ratios = parse_paired_ratios(args.h2, "8c256")
    browser_full = parse_throughput_full(args.browser, "8c256")
    browser_ratios = parse_paired_ratios(args.browser, "8c256")
    paced_full = parse_paced_full(args.paced, "c8x256")
    h1_full = parse_throughput_full(args.h1, "64par")

    h2 = {k: v[0] for k, v in h2_full.items()}
    browser = {k: v[0] for k, v in browser_full.items()}
    cpu = {k: v[1] for k, v in paced_full.items()}
    h1 = {k: v[0] for k, v in h1_full.items()}

    args.out_dir.mkdir(parents=True, exist_ok=True)
    for name, theme in THEMES.items():
        target = args.out_dir / f"bench-{name}.svg"
        target.write_text(render_svg(theme, h2, browser, cpu, h1))
        print(f"wrote {target}")

    if args.social_html:
        args.social_html.parent.mkdir(parents=True, exist_ok=True)
        args.social_html.write_text(
            render_social_html(
                args.version,
                h2_full,
                h2_ratios,
                browser_full,
                browser_ratios,
                paced_full,
                h1_full,
            )
        )
        print(f"wrote {args.social_html}")


if __name__ == "__main__":
    main()
