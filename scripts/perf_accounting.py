#!/usr/bin/env python3
"""Per-request performance accounting for a comparison client and its origin.

Runs a comparison client against a local origin under `perf stat` counters
for both sides, then prints cycles, instructions, task-clock, context
switches, and page faults per completed operation. Optional modes add BPF
syscall counts and `perf record` call graphs.

    scripts/perf_accounting.py --client leyline --preset small
    scripts/perf_accounting.py --client wreq --preset large --syscalls
    scripts/perf_accounting.py --client leyline --preset small --record

The counters explain where a throughput difference lives. They do not rank
clients on their own; pair them with `benches/comparison/paired.sh` for
throughput and `benches/comparison/netem.sh` for wide-area behavior.

Requirements: Linux, perf (passwordless sudo), optionally bpftrace for
--syscalls. Binaries live in benches/comparison/bin.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shlex
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BENCH = ROOT / "benches/comparison"
BIN = BENCH / "bin"
OUT = Path(os.environ.get("PERF_OUT", Path("/tmp/leyline-perf-accounting")))

PRESETS = {
    "small": dict(body=10, warm=100000, cold=500, requests=524288, concurrency=64),
    "medium": dict(body=16384, warm=65536, cold=200, requests=262144, concurrency=64),
    "large": dict(body=4194304, warm=1024, cold=64, requests=1024, concurrency=8),
}
EVENTS = "cycles,instructions,task-clock,context-switches,cpu-migrations,page-faults"
SYSCALL_PROBE = (
    "tracepoint:syscalls:sys_exit_sendto,tracepoint:syscalls:sys_exit_sendmsg,"
    "tracepoint:syscalls:sys_exit_write,tracepoint:syscalls:sys_exit_writev,"
    "tracepoint:syscalls:sys_exit_recvfrom,tracepoint:syscalls:sys_exit_recvmsg,"
    "tracepoint:syscalls:sys_exit_read,tracepoint:syscalls:sys_exit_readv,"
    "tracepoint:syscalls:sys_exit_epoll_wait,tracepoint:syscalls:sys_exit_epoll_pwait,"
    "tracepoint:syscalls:sys_exit_futex"
)


def run(args: list[str], **kw) -> subprocess.CompletedProcess:
    return subprocess.run(args, capture_output=True, text=True, **kw)


def sudo(args: list[str], **kw) -> subprocess.CompletedProcess:
    return run(["sudo", "-n", *args], **kw)


def perf_json(path: Path) -> dict[str, float]:
    values: dict[str, float] = {}
    if not path.exists():
        return values
    for line in path.read_text().splitlines():
        try:
            entry = json.loads(line)
        except json.JSONDecodeError:
            continue
        values[entry["event"]] = float(entry["counter-value"])
    return values


def syscall_counts(path: Path) -> dict[str, int]:
    counts: dict[str, int] = {}
    if not path.exists():
        return counts
    for line in path.read_text().splitlines():
        try:
            entry = json.loads(line)
        except json.JSONDecodeError:
            continue
        if entry.get("type") != "map":
            continue
        for key, value in entry["data"]["@calls"].items():
            name, ret = key.rsplit(",", 1)
            if int(ret) >= 0:
                name = name.split(":")[-1].removeprefix("sys_exit_")
                counts[name] = counts.get(name, 0) + value
    return counts


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--client", default="leyline", help="binary name in benches/comparison/bin")
    parser.add_argument("--origin", default="server", help="origin binary name")
    parser.add_argument("--preset", choices=sorted(PRESETS), default="small")
    parser.add_argument("--rounds", type=int, default=2)
    parser.add_argument("--body", type=int)
    parser.add_argument("--warm", type=int)
    parser.add_argument("--cold", type=int)
    parser.add_argument("--requests", type=int)
    parser.add_argument("--concurrency", type=int)
    parser.add_argument("--syscalls", action="store_true", help="count syscalls with bpftrace")
    parser.add_argument("--record", action="store_true", help="capture call graphs with perf record")
    parser.add_argument("--client-cpus", default="2-15")
    parser.add_argument("--origin-cpus", default="0-1")
    args = parser.parse_args()

    settings = dict(PRESETS[args.preset])
    for field in ("body", "warm", "cold", "requests", "concurrency"):
        if getattr(args, field) is not None:
            settings[field] = getattr(args, field)

    client = BIN / args.client
    origin = BIN / args.origin
    for path in (client, origin):
        if not path.exists():
            print(f"ERROR: missing {path}", file=sys.stderr)
            return 1

    OUT.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env.update(
        CMP_BODY=str(settings["body"]),
        LEYLINE_CHROME=env.get("LEYLINE_CHROME", "149"),
        CMP_TLS="matched",
        TOKIO_WORKER_THREADS="12",
    )
    env.pop("CMP_HEADERS", None)

    if args.syscalls and args.record:
        parser.error("--syscalls and --record cannot be combined")

    log_path = OUT / "origin.log"
    log = log_path.open("w")
    origin_env = env.copy()
    origin_env["GOMAXPROCS"] = "2"
    origin_proc = subprocess.Popen(
        ["taskset", "-c", args.origin_cpus, str(origin), "127.0.0.1:0"],
        stdout=log, stderr=log, env=origin_env,
    )
    try:
        url = None
        for _ in range(100):
            match = re.search(r"https://[0-9.]+:[0-9]+/", log_path.read_text() or "")
            if match:
                url = match.group(0)
                break
            if origin_proc.poll() is not None:
                print("ERROR: origin exited:\n" + log_path.read_text()[-800:], file=sys.stderr)
                return 1
            time.sleep(0.1)
        if url is None:
            print("ERROR: origin never listened", file=sys.stderr)
            return 1
        print(f"== {args.client} vs {args.origin} at {url} body={settings['body']} "
              f"requests={settings['requests']} concurrency={settings['concurrency']}", file=sys.stderr)

        operations = settings["warm"] + settings["cold"] + settings["requests"]
        rows = []
        for round_index in range(1, args.rounds + 1):
            server_stat = OUT / f"round{round_index}-server.json"
            client_stat = OUT / f"round{round_index}-client.json"
            recorder = subprocess.Popen(
                ["sudo", "-n", "perf", "stat", "-j", "-e", EVENTS, "-p", str(origin_proc.pid), "-o", str(server_stat)],
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            )
            time.sleep(0.3)
            inner = ["/usr/bin/env", "-i", "PATH=/usr/bin:/bin"]
            inner += [f"{key}={env[key]}" for key in ("LEYLINE_CHROME", "TOKIO_WORKER_THREADS", "CMP_BODY", "CMP_TLS", "CMP_CA")
                      if env.get(key)]
            inner += ["taskset", "-c", args.client_cpus, str(client), url,
                      str(settings["warm"]), str(settings["cold"]),
                      str(settings["requests"]), str(settings["concurrency"])]
            client_cmd = ["sudo", "-n", "perf", "stat", "-j", "-e", EVENTS, "-o", str(client_stat), "--", *inner]
            if args.syscalls:
                probe = OUT / f"round{round_index}-client.bt"
                probe.write_text(SYSCALL_PROBE + " /pid == cpid/ { @calls[probe, args.ret] = count(); }\n")
                maps = OUT / f"round{round_index}-client-syscalls.jsonl"
                client_cmd = ["sudo", "-n", "bpftrace", "-f", "json", "-o", str(maps), "-c", shlex.join(inner), str(probe)]
            if args.record:
                client_data = OUT / f"round{round_index}-client.data"
                client_cmd = ["sudo", "-n", "perf", "record", "-F", "199", "--call-graph", "dwarf,8192",
                              "-o", str(client_data), "--", *inner]
            result = run(client_cmd, env=env, timeout=600)
            sudo(["kill", "-INT", str(recorder.pid)])
            try:
                recorder.wait(timeout=5)
            except subprocess.TimeoutExpired:
                recorder.kill()
                recorder.wait()
            assert result.returncode == 0 and "conc_rps" in result.stdout, result.stdout[-400:] + result.stderr[-400:]
            if args.record:
                report = sudo(["perf", "report", "--stdio", "--no-children", "--percent-limit", "0.5",
                               "-i", str(client_data)], check=True)
                (OUT / f"round{round_index}-client.txt").write_text(report.stdout)
            values = {k: float(v) for k, v in re.findall(r"(\w+)=([\d.]+)", result.stdout)}
            rows.append({"round": round_index, "result": values,
                         "client": perf_json(client_stat), "server": perf_json(server_stat)})
            client_rates = f"conc={values.get('conc_rps', 0):.0f}"
            print(f"round {round_index}: {client_rates}", file=sys.stderr)

        print(f"\n== per-operation accounting (mean of {args.rounds} rounds, operations={operations}) ==")
        for side in ("client", "server"):
            print(f"\n{side}:")
            for event in ("cycles", "instructions", "task-clock", "context-switches", "page-faults"):
                values = [row[side].get(event, 0) for row in rows]
                mean = sum(values) / len(values)
                per_op = mean / operations
                unit = "ms" if event == "task-clock" else ""
                print(f"  {event:>18}: {mean:>14.0f}{unit}  ({per_op:.3f}/op)")
        for row in rows:
            for path in OUT.glob(f"round{row['round']}-*"):
                if path.name.endswith(("-server.json", "-client.json")):
                    continue
                if "syscalls" not in path.name:
                    continue
                counts = syscall_counts(path)
                if counts:
                    print(f"\nsyscalls round {row['round']}:")
                    for name, count in sorted(counts.items()):
                        print(f"  {name:>12}: {count:>10}  ({count / operations:.3f}/op)")
        raw = OUT / "accounting.json"
        raw.write_text(json.dumps({"client": args.client, "origin": args.origin,
                                   "settings": settings, "ops": operations, "rounds": rows}, indent=2) + "\n")
        print(f"\nraw counters: {raw}")
        return 0
    finally:
        origin_proc.terminate()
        try:
            origin_proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            origin_proc.kill()
            origin_proc.wait()
        log.close()


if __name__ == "__main__":
    raise SystemExit(main())
