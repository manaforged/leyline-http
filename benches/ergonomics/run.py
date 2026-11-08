import argparse
import hashlib
import json
import os
import re
import shutil
import statistics
import subprocess
import sys
import time
import tomllib
from pathlib import Path

import fixture
import score

ROOT = Path(__file__).resolve().parent
WORK = ROOT / "work"
TASKS = [f"t{n:02d}" for n in range(1, 11)]
TASK_NAMES = {
    "t01": "GET and parse JSON",
    "t02": "POST JSON with a timeout",
    "t03": "Session cookie",
    "t04": "Retry with backoff and Retry-After",
    "t05": "Fan out 100, 10 in flight",
    "t06": "Resumable download",
    "t07": "Multipart upload",
    "t08": "SSE with reconnect",
    "t09": "HTTP proxy",
    "t10": "Redirect cap and final URL",
}
B = fixture.BASE
EXPECTED = {
    "t01": "42 ada",
    "t02": "201 7 timeout",
    "t03": "ada",
    "t04": "200 3",
    "t05": "90 10",
    "t06": str(fixture.LARGE_SIZE),
    "t07": f"200 {fixture.UPLOAD_SIZE} hello",
    "t08": "a b c",
    "t09": "200 proxy",
    "t10": f"{B}/t10/final capped",
}
TIMEOUT = {"t06": 240}
EXT = {"rust": "rs", "python": "py", "node": "mjs"}
ORDER = {"rust": 0, "python": 1, "node": 2}


def libraries():
    data = tomllib.loads((ROOT / "libraries.toml").read_text())["library"]
    return sorted(data, key=lambda lib: ORDER[lib["lang"]])


def source_path(lib, task):
    name = lib["name"]
    if lib["lang"] == "rust":
        return ROOT / "rust" / name / "src" / "bin" / f"{task}.rs"
    return ROOT / lib["lang"] / name / f"{task}.{EXT[lib['lang']]}"


def clean_env():
    env = {k: v for k, v in os.environ.items() if "proxy" not in k.lower()}
    env["NO_COLOR"] = "1"
    return env


def scrub(text):
    text = text.replace(str(ROOT), "$ROOT")
    return re.sub(r"(/home|/Users)/[^/\s]+", "~", text)


def build_rust(lib):
    target = WORK / "target" / lib["name"]
    project = ROOT / "rust" / lib["name"]
    result = subprocess.run(
        ["nice", "-n", "19", "cargo", "build", "--release", "--bins", "--keep-going"],
        cwd=project,
        env={**clean_env(), "CARGO_BUILD_JOBS": "4", "CARGO_TARGET_DIR": str(target)},
        capture_output=True,
        text=True,
    )
    return target / "release", scrub(result.stderr)


def command(lib, task, binaries, python):
    if lib["lang"] == "rust":
        return [str(binaries / task)]
    if lib["lang"] == "python":
        return [python, str(source_path(lib, task))]
    return ["node", str(source_path(lib, task))]


def run_program(server, lib, task, cmd, node_path):
    cwd = WORK / "cwd"
    shutil.rmtree(cwd, ignore_errors=True)
    cwd.mkdir(parents=True)
    (cwd / "upload.bin").write_bytes(fixture.UPLOAD)
    server.reset()
    env = clean_env()
    if lib["lang"] == "node":
        env["NODE_PATH"] = node_path
    started = time.monotonic()
    try:
        done = subprocess.run(
            cmd,
            cwd=cwd,
            env=env,
            capture_output=True,
            text=True,
            timeout=TIMEOUT.get(task, 60),
            stdin=subprocess.DEVNULL,
        )
        code, stdout, stderr = done.returncode, done.stdout, done.stderr
    except subprocess.TimeoutExpired as err:
        code = "timeout"
        stdout = (err.stdout or b"").decode(errors="replace")
        stderr = (err.stderr or b"").decode(errors="replace")
    ended = time.monotonic()
    state = server.state
    time.sleep(0.05)
    output = " ".join(stdout.split())
    checks = fixture.check(task, state, ended)
    if task == "t06":
        target = cwd / "large.bin"
        digest = hashlib.sha256(target.read_bytes()).hexdigest() if target.exists() else None
        checks["file_hash"] = digest == fixture.LARGE_SHA
    output_ok = output == EXPECTED[task]
    passed = code == 0 and output_ok and all(checks.values())
    return {
        "passed": passed,
        "exit": code,
        "output": output[:300],
        "output_ok": output_ok,
        "server_checks": checks,
        "server_facts": fixture.facts(task, state),
        "seconds": round(ended - started, 2),
        "stderr_tail": scrub(stderr[-600:]) if not passed else "",
    }


def load_surface(lib):
    path = WORK / "surface" / lib["lang"] / f"{lib['name']}.json"
    data = json.loads(path.read_text()) if path.exists() else {}
    return {name: set(values) for name, values in data.items()}


def programs(args):
    server = fixture.Fixture()
    server.start()
    python = args.python or sys.executable
    results_path = ROOT / "results.json"
    results = json.loads(results_path.read_text()) if results_path.exists() and args.merge else {}
    node_path = str(ROOT / "node" / "node_modules")
    for lib in libraries():
        if args.lib and lib["name"] not in args.lib.split(","):
            continue
        if args.lang and lib["lang"] != args.lang:
            continue
        binaries = None
        if lib["lang"] == "rust":
            binaries, log = build_rust(lib)
            (WORK / f"build-{lib['name']}.log").write_text(log)
        surface = load_surface(lib)
        key = f"{lib['lang']}/{lib['name']}"
        entry = results.setdefault(key, {})
        for task in TASKS:
            if args.task and task not in args.task.split(","):
                continue
            path = source_path(lib, task)
            if not path.exists():
                entry[task] = {"passed": False, "missing": True}
                continue
            project = ROOT / "rust" / lib["name"] if lib["lang"] == "rust" else None
            metrics = score.score(path, lib["lang"], lib["name"], set(lib["bundled"]), surface, project)
            cmd = command(lib, task, binaries, python)
            if lib["lang"] == "rust" and not Path(cmd[0]).exists():
                outcome = {"passed": False, "exit": "no binary", "output": "", "stderr_tail": "compile failed; see work/build log"}
            else:
                outcome = run_program(server, lib, task, cmd, node_path)
            entry[task] = {**metrics, **outcome}
            mark = "ok  " if outcome["passed"] else "FAIL"
            print(
                f"{mark} {key} {task} tok={metrics['tokens']} out={outcome.get('output', '')!r} "
                f"checks={ {k: v for k, v in outcome.get('server_checks', {}).items() if not v} }",
                flush=True,
            )
    results_path.write_text(json.dumps(results, indent=1, sort_keys=True) + "\n")
    server.shutdown()
    write_table(results)


def write_table(results):
    libs = libraries()
    footprint_path = ROOT / "footprint.json"
    footprint = json.loads(footprint_path.read_text()) if footprint_path.exists() else {}
    out = ["# Ergonomics results", ""]
    out.append("Generated by `run.py`. Cell: tokens/lines. `F` means the program failed its check. `*` marks the fewest tokens among passing programs in that row.")
    out.append("")
    for lang in ("rust", "python", "node"):
        group = [lib for lib in libs if lib["lang"] == lang]
        names = [lib["name"] for lib in group]
        out.append(f"## {lang}")
        out.append("")
        out.append("| Task | " + " | ".join(names) + " | leyline rank |")
        out.append("| --- |" + " --- |" * (len(names) + 1))
        home = names[0]
        wins = dict.fromkeys(names, 0)
        for task in TASKS:
            row = {name: results.get(f"{lang}/{name}", {}).get(task, {}) for name in names}
            passing = {n: r["tokens"] for n, r in row.items() if r.get("passed")}
            best = min(passing.values()) if passing else None
            cells = []
            for name in names:
                r = row[name]
                if not r.get("passed"):
                    cells.append(f"F {r.get('tokens', '-')}/{r.get('lines', '-')}")
                    continue
                star = "*" if r["tokens"] == best else ""
                wins[name] += r["tokens"] == best
                cells.append(f"{star}{r['tokens']}/{r['lines']}")
            rank = 1 + sum(1 for value in passing.values() if home in passing and value < passing[home])
            shown = f"{rank} of {len(names)}" if home in passing else "F"
            out.append(f"| {task} {TASK_NAMES[task]} | " + " | ".join(cells) + f" | {shown} |")
        for label, field in (("tokens", "tokens"), ("lines", "lines"), ("imports", "imports"), ("extra deps", "extra_deps"), ("concepts", "concepts")):
            cells = []
            for name in names:
                values = [r[field] for r in results.get(f"{lang}/{name}", {}).values() if field in r]
                cells.append(str(sum(values)))
            out.append(f"| total {label} | " + " | ".join(cells) + " | |")
        out.append("| passed | " + " | ".join(str(sum(1 for r in results.get(f"{lang}/{n}", {}).values() if r.get("passed"))) for n in names) + " | |")
        out.append("| fewest-token wins | " + " | ".join(str(wins[n]) for n in names) + " | |")
        out.append("")
    if footprint:
        out.append("## Install and build footprint")
        out.append("")
        out.append("| Library | Install | Cold build median s (3 runs) | Runs s | Crates compiled | Release binary MiB | Stripped MiB | Installed MiB |")
        out.append("| --- | --- | --- | --- | --- | --- | --- | --- |")
        for lib in libs:
            key = f"{lib['lang']}/{lib['name']}"
            row = footprint.get(key, {})
            runs = row.get("cold_build_seconds")
            median = f"{statistics.median(runs):.0f}" if runs else "-"
            spread = ", ".join(f"{r:.0f}" for r in runs) if runs else "-"
            binary = f"{row['binary_bytes'] / 1048576:.1f}" if "binary_bytes" in row else "-"
            installed = f"{row['installed_bytes'] / 1048576:.1f}" if "installed_bytes" in row else "-"
            stripped = f"{row['stripped_bytes'] / 1048576:.1f}" if "stripped_bytes" in row else "-"
            crates = row.get("crates_compiled", "-")
            out.append(f"| {key} | `{lib['install']}` | {median} | {spread} | {crates} | {binary} | {stripped} | {installed} |")
        out.append("")
    (ROOT / "results.md").write_text("\n".join(out))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--lang")
    parser.add_argument("--lib")
    parser.add_argument("--task")
    parser.add_argument("--python")
    parser.add_argument("--merge", action="store_true")
    parser.add_argument("--table", action="store_true")
    args = parser.parse_args()
    if args.table:
        write_table(json.loads((ROOT / "results.json").read_text()))
        return
    WORK.mkdir(exist_ok=True)
    programs(args)


if __name__ == "__main__":
    main()
