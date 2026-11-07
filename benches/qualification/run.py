#!/usr/bin/env python3
"""Build and run the qualification scenarios. Results and pass stamps go under results/<tier>/."""
import argparse
import concurrent.futures
import dataclasses
import hashlib
import json
import os
import pathlib
import queue
import re
import resource
import shutil
import socket
import subprocess
import sys
import time

HERE = pathlib.Path(__file__).resolve().parent
SLOT_CPUS = 4
BASE_PORT = 20000
PORTS_PER_SLOT = 5000
READY_SECONDS = 20
GRACE_SECONDS = 60
NETEM_QUEUE_PACKETS = 10_000_000
NGINX_UPSTREAM_OFFSET = 1000
SANITIZER_OPTIONS = ("ASAN_OPTIONS", "TSAN_OPTIONS")
NAMESPACE_SETUP = (
    "ip link set lo up; "
    "sysctl -qw net.core.somaxconn=65535 net.ipv4.tcp_max_syn_backlog=65535 "
    "net.ipv4.ip_local_port_range='10000 65000' net.ipv4.tcp_max_tw_buckets=2000000; "
    "ulimit -n 1000000; "
)


@dataclasses.dataclass
class Scenario:
    name: str
    command: list
    seconds: int
    origin: list = dataclasses.field(default_factory=list)
    expect: str = "pass"
    netem: str = ""
    client_cpus: int = 0
    behind_nginx: bool = False


def edge(name, mode, rate, hosts, client="", origin="", seconds=30, bad=(20, 50), expect="pass"):
    bad_flags = f"--bad-first {bad[0]} --bad-count {bad[1]}".split()
    return Scenario(
        name,
        ["edge", "--mode", mode, "--rate", str(rate), "--hosts", str(hosts), *bad_flags, *client.split()],
        seconds,
        ["origin", "--seed", "1", *bad_flags, *origin.split()],
        expect,
    )


def hostile(attack, seconds):
    return Scenario(
        f"hostile-{attack}",
        ["hostile", "--attack", attack, "--ca", "{ca}"],
        seconds,
        ["hostile-origin", "--attack", attack, "--ca", "{ca}"],
    )


def mux(name, seconds, origin, client):
    return Scenario(f"mux-{name}", [*client.split(), "--ca", "{ca}"], seconds, ["mux-origin", "--ca", "{ca}", *origin.split()])


def life(name, case, flags="", seconds=0, expect="pass"):
    return Scenario(f"life-{name}", ["life", "--case", case, *flags.split()], seconds, expect=expect)


NO_BAD = (0, 0)
TOP_HOST = (0, 1)
ATTACKS = (
    "none ping settings empty-data empty-continuation growing-continuation rapid-reset "
    "zero-window window-flood huge-headers slow-body goaway-loop data-flood"
).split()


def quick():
    loose = "--min-reuse 0.9 --p99-ratio 20 --max-rss-growth 2 --deadline-ms 1000"
    shed = "--capacity-permits 10 --service-us 1000 --queue-limit 20"
    fast = 6
    return [
        edge("bad-hosts", "shared", 20000, 2000, f"--max-pending 1024 {loose}", seconds=fast),
        edge("bad-hosts-cores", "cores", 20000, 2000, f"--max-pending 1024 {loose}", seconds=fast),
        edge("many-hosts", "shared", 20000, 20000, f"--max-pending 1024 --max-connections 50000 {loose}", seconds=fast),
        edge("stalled-host", "shared", 20000, 2000, f"--max-pending 1024 {loose}", "--stall-only", fast, TOP_HOST),
        edge("churn-announced", "shared", 10000, 200, f"--retries 2 --max-fds 5000 {loose.replace('--min-reuse 0.9', '--min-reuse -1')}", "--close-every 1", fast, NO_BAD),
        edge("churn-silent", "shared", 10000, 200, f"--retries 2 --max-fds 5000 {loose.replace('--min-reuse 0.9', '--min-reuse -1')}", "--close-every 10 --close-silently", fast, NO_BAD),
        edge("overload-budget", "shared", 40000, 2000, "--overload --retries 2 --retry-budget 0.2 --backoff-min-ms 100 --backoff-max-ms 1000 --deadline-ms 1000", shed, fast, NO_BAD),
        edge("overload-limit", "shared", 40000, 2000, "--overload --retries 0 --deadline-ms 1000 --max-total 10 --max-total-pending 10", "--capacity-permits 10 --service-us 1000", fast, NO_BAD),
        *[hostile(attack, 3) for attack in ATTACKS],
        Scenario("h2-cold-burst", ["hostile", "--attack", "none", "--ca", "{ca}", "--burst", "3000", "--rate", "100"], 3, ["hostile-origin", "--attack", "none", "--ca", "{ca}"]),
        mux("goaway", fast, "--hosts 200 --restart-at 3", "reconnect --hosts 200 --rate 5000"),
        mux("abrupt", fast, "--hosts 200 --restart-at 3 --abrupt", "reconnect --hosts 200 --rate 5000"),
        mux("idle", fast, "--hosts 1", "idle --rate 2000 --streams 5000"),
        life("bodies", "bodies", "--max-bytes 67108864"),
        life("slow-readers", "slow-readers", "--readers 200 --bytes 1048576"),
        life("failures", "failures"),
        life("dns-move", "dns-move", "--max-lifetime-ms 200 --settle-ms 500 --hold-ms 500"),
        life("capacity-loss", "capacity-loss", "--settle-ms 500 --hold-ms 1000"),
        life("certificate-rotation", "certificate-rotation", "--hold-ms 1000"),
        life("shutdown", "shutdown", "--hold-ms 1000"),
        dataclasses.replace(edge("delay-and-loss", "shared", 5000, 200, f"--retries 2 {loose} --deadline-ms 3000".replace("--deadline-ms 1000 ", ""), seconds=fast, bad=NO_BAD), netem="delay 5ms loss 0.1%"),
        dataclasses.replace(edge("behind-nginx", "shared", 5000, 200, f"--max-pending 1024 {loose.replace('--min-reuse 0.9', '--min-reuse -1')}", seconds=fast), behind_nginx=True),
        Scenario("chaos", ["chaos", "--seed", "1"], fast),
        Scenario("cancel", ["cancel", "--seed", "1"], fast),
    ]


def full():
    limit = "--max-pending 1024"
    capacity = "--capacity-permits 100 --service-us 1000"
    shed = f"{capacity} --queue-limit 200"
    backoff = "--backoff-min-ms 100 --backoff-max-ms 1000"
    overload = "--overload --retries 0 --deadline-ms 1000"
    total = "--max-total 100 --max-total-pending 100"
    plans = [
        edge("q10-20k", "shared", 20000, 2000, f"{limit} --min-reuse 0.998"),
        edge("q10-100k", "shared", 100000, 2000, limit),
        edge("q10-300k", "shared", 300000, 2000, limit),
        edge("q10-300k-deadline-5s", "shared", 300000, 2000, f"{limit} --deadline-ms 5000", expect="any"),
        edge("q10-cores-300k", "cores", 300000, 2000, f"{limit} --min-reuse 0.99"),
        edge("q11-20k-hosts", "shared", 300000, 20000, f"{limit} --max-connections 50000"),
        edge("q11-200k-hosts", "shared", 300000, 200000, f"{limit} --max-connections 400000"),
        edge("q11-default-limit", "shared", 300000, 20000, limit, expect="fail"),
        edge("q15-100k", "shared", 100000, 2000, limit, "--stall-only", bad=TOP_HOST),
        edge("q15-300k", "shared", 300000, 2000, limit, "--stall-only", bad=TOP_HOST),
        edge("q3-announced", "shared", 50000, 2000, "--retries 2 --min-reuse -1 --max-fds 20000", "--close-every 1", bad=NO_BAD),
        edge("q3-silent", "shared", 50000, 2000, "--retries 2 --min-reuse -1 --max-fds 20000", "--close-every 100 --close-silently", bad=NO_BAD),
    ]
    for rate in (40000, 80000, 120000, 200000, 400000):
        plans.append(edge(f"q17-sheds-{rate}", "shared", rate, 2000, overload, shed, 20, NO_BAD))
        plans.append(edge(f"q17-limit-{rate}", "shared", rate, 2000, f"{overload} {total}", capacity, 20, NO_BAD))
    plans += [
        edge("q17-fifo", "shared", 120000, 2000, overload, capacity, 20, NO_BAD, "fail"),
        edge("q17-no-budget", "shared", 400000, 2000, f"--overload --retries 2 {backoff} --deadline-ms 5000", shed, 20, NO_BAD, "fail"),
        edge("q17-budget", "shared", 400000, 2000, f"--overload --retries 2 --retry-budget 0.2 {backoff} --deadline-ms 5000", shed, 20, NO_BAD),
        edge("q18-sheds", "shared", 40000, 2000, "--overload --shock 3 --retries 3 --deadline-ms 1000", shed, 40, NO_BAD),
        edge("q18-fifo", "shared", 40000, 2000, "--overload --shock 3 --retries 0 --deadline-ms 1000", capacity, 40, NO_BAD, "fail"),
        edge("q18-fifo-limit", "shared", 40000, 2000, f"--overload --shock 3 --retries 3 --deadline-ms 1000 {total}", capacity, 40, NO_BAD),
        edge("q16", "shared", 40000, 2000, f"--overload --shock 2.5 --retries 2 --retry-budget 0.2 {backoff} --deadline-ms 1000 --max-rss-mib 2048", shed, 300, NO_BAD),
    ]
    for round_number in (1, 2):
        for label, hosts, extra in (("2k", 2000, ""), ("20k", 20000, "--max-connections 50000"), ("200k", 200000, "--max-connections 400000")):
            for mode in ("shared", "pingora"):
                plans.append(edge(f"q27-{label}-{mode}-r{round_number}", mode, 300000, hosts, f"--retries 0 {extra}", seconds=20, expect="any"))
            plans.append(edge(f"q27-{label}-limit-r{round_number}", "shared", 300000, hosts, f"--retries 0 {extra} {limit}", seconds=20, expect="any"))
        for mode, extra in (("pingora", ""), ("shared", ""), ("shared", limit)):
            tag = "limit" if extra else mode
            plans.append(edge(f"q27-stall-{tag}-r{round_number}", mode, 100000, 2000, f"--retries 0 {extra}", "--stall-only", 20, TOP_HOST, "any"))
    for rate in (400000, 500000, 600000, 700000):
        plans.append(edge(f"q1-{rate}", "shared", rate, 2000, limit, seconds=20, bad=NO_BAD, expect="any"))
    for cores in (1, 2, 4, 8, 16):
        plans.append(dataclasses.replace(edge(f"q12-{cores}-cpus", "shared", 40000 * cores, 2000, limit, seconds=20, bad=NO_BAD, expect="any"), client_cpus=cores))
    for label, netem, rate in (("lan", "delay 1ms loss 0.01%", 50000), ("metro", "delay 5ms loss 0.1%", 50000), ("wan", "delay 40ms loss 1%", 20000)):
        plans.append(dataclasses.replace(edge(f"q7-{label}", "shared", rate, 2000, f"--retries 2 {limit} --min-reuse 0.9 --p99-ratio 5", seconds=20, bad=NO_BAD), netem=netem))
    plans.append(dataclasses.replace(edge("q28-nginx", "shared", 20000, 2000, "--max-pending 1024 --min-reuse -1 --p99-ratio 5"), behind_nginx=True))
    plans += [hostile(attack, 15) for attack in ATTACKS]
    plans.append(Scenario("h2-cold-burst", ["hostile", "--attack", "none", "--ca", "{ca}", "--burst", "20000", "--rate", "100"], 15, ["hostile-origin", "--attack", "none", "--ca", "{ca}"]))
    plans += [
        mux("goaway-2000", 30, "--hosts 2000 --restart-at 16", "reconnect --hosts 2000 --rate 100000"),
        mux("abrupt-2000", 30, "--hosts 2000 --restart-at 16 --abrupt", "reconnect --hosts 2000 --rate 100000"),
        mux("idle-100k", 45, "--hosts 1", "idle --rate 20000 --streams 100000"),
        life("bodies", "bodies"),
        life("slow-readers", "slow-readers"),
        life("failures", "failures"),
        life("dns-move-no-lifetime", "dns-move", expect="fail"),
        life("dns-move", "dns-move", "--max-lifetime-ms 5000"),
        life("capacity-loss", "capacity-loss"),
        life("certificate-rotation", "certificate-rotation"),
        life("shutdown", "shutdown"),
    ]
    return plans


def soak():
    return [edge("q2", "shared", 100000, 2000, "--max-pending 1024", seconds=3600, bad=NO_BAD)]


TIERS = {"quick": quick, "full": full, "soak": soak}
PROFILES = {"quick": "quick", "full": "release", "soak": "release"}


def wait_ready(ports, origin):
    deadline = time.monotonic() + READY_SECONDS
    waiting = list(ports)
    while waiting and time.monotonic() < deadline:
        if origin.poll() is not None:
            return False
        try:
            socket.create_connection(("127.0.0.1", waiting[-1]), timeout=0.2).close()
            waiting.pop()
        except OSError:
            time.sleep(0.005)
    return not waiting


def origin_ports(port, arguments):
    if arguments[0] == "hostile-origin":
        return [port + 1]
    if arguments[0] == "mux-origin" and "--hosts" in arguments:
        return range(port, port + int(arguments[arguments.index("--hosts") + 1]))
    return [port]


NGINX_CONFIG = """
daemon off;
worker_processes {workers};
pid {base}.pid;
error_log {base}.nginx.log error;
worker_rlimit_nofile 200000;
events {{ worker_connections 100000; }}
http {{
    access_log off;
    client_body_temp_path {base}.temp;
    proxy_temp_path {base}.temp;
    fastcgi_temp_path {base}.temp;
    uwsgi_temp_path {base}.temp;
    scgi_temp_path {base}.temp;
    keepalive_requests 10000000;
    keepalive_timeout 300s;
    server {{
        listen {port} reuseport;
        location / {{
            proxy_pass http://$server_addr:{upstream};
            proxy_http_version 1.1;
            proxy_set_header Connection "";
            proxy_set_header Host $host;
            proxy_read_timeout 60s;
        }}
    }}
}}
"""


def start_nginx(job, port, upstream):
    base = job["out"]
    config = pathlib.Path(base + ".nginx.conf")
    config.write_text(NGINX_CONFIG.format(workers=len(job["origin_cpus"]), base=base, port=port, upstream=upstream))
    arguments = ["-p", str(config.parent), "-c", str(config), "-e", base + ".nginx.log"]
    return subprocess.Popen(["taskset", "-c", ",".join(map(str, job["origin_cpus"])), job["nginx"], *arguments], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def pinned(cpus, binary, arguments):
    return ["taskset", "-c", ",".join(map(str, cpus)), binary, *arguments]


def run_job(job):
    os.environ.update(job["env"])
    hard = resource.getrlimit(resource.RLIMIT_NOFILE)[1]
    resource.setrlimit(resource.RLIMIT_NOFILE, (hard, hard))
    port, out = job["port"], job["out"]
    fill = lambda part: part.replace("{ca}", out + ".ca")
    origin = None
    nginx = None
    log = open(out + ".log", "w")
    command = [fill(a) for a in job["command"]]
    if job["origin"]:
        origin_port = port + NGINX_UPSTREAM_OFFSET if job["nginx"] else port
        flags = ["--port", str(origin_port), "--origin-threads", str(len(job["origin_cpus"]))]
        origin = subprocess.Popen(pinned(job["origin_cpus"], job["binary"], [*map(fill, job["origin"]), *flags]), stdout=subprocess.DEVNULL, stderr=log)
        if not wait_ready(origin_ports(origin_port, job["origin"]), origin):
            origin.kill()
            print("origin did not become ready", file=log)
            return 3
        if job["nginx"]:
            nginx = start_nginx(job, port, origin_port)
            if not wait_ready([port], nginx):
                nginx.kill()
                origin.kill()
                print("nginx did not become ready", file=log)
                return 3
        command += ["--port", str(port)]
    command += ["--out", out]
    if job["seconds"]:
        command += ["--seconds", str(job["seconds"])]
    try:
        cpus = job["client_cpus"] if job["origin"] else job["client_cpus"] + job["origin_cpus"]
        env = {**os.environ, "LD_PRELOAD": job["preload"]} if job["preload"] else None
        return subprocess.run(pinned(cpus, job["binary"], command), stdout=subprocess.DEVNULL, stderr=log, timeout=job["seconds"] + GRACE_SECONDS, stdin=subprocess.DEVNULL, env=env).returncode
    except subprocess.TimeoutExpired:
        print("client timed out", file=log)
        return 4
    finally:
        for process in (nginx, origin):
            if process:
                process.kill()
                process.wait()


def isolated(job, netem):
    """Runs the job in a private network namespace when sudo allows it."""
    if not shutil.which("unshare") or subprocess.run(["sudo", "-n", "true"], capture_output=True).returncode:
        if netem:
            raise SystemExit("delay and loss scenarios need sudo and unshare")
        return run_job(job)
    shape = f"tc qdisc add dev lo root netem limit {NETEM_QUEUE_PACKETS} {netem}; " if netem else ""
    back = f"exec setpriv --reuid={os.getuid()} --regid={os.getgid()} --init-groups"
    script = f"{NAMESPACE_SETUP}{shape}{back} {sys.executable} {__file__} --job \"$1\""
    return subprocess.run(["sudo", "-n", "unshare", "-n", "bash", "-c", script, "job", json.dumps(job)], stdin=subprocess.DEVNULL).returncode


def outcome_of(code, out):
    if not pathlib.Path(out).exists() or code not in (0, 1):
        return "crash"
    return "pass" if code == 0 else "fail"


def execute(scenario, binary, key, folder, slots, preload, nginx_path):
    stamp = folder / f"{scenario.name}.stamp"
    if stamp.exists() and stamp.read_text() == key:
        return scenario, "cached", 0.0, []
    origin_cpus, client_cpus, slot = slots.get()
    out = str(folder / f"{scenario.name}.json")
    pathlib.Path(out).unlink(missing_ok=True)
    began = time.monotonic()
    try:
        code = isolated({
            "binary": binary, "command": scenario.command, "origin": scenario.origin, "seconds": scenario.seconds,
            "origin_cpus": origin_cpus, "client_cpus": client_cpus[:scenario.client_cpus or None], "port": BASE_PORT + slot * PORTS_PER_SLOT, "out": out, "preload": preload, "nginx": nginx_path if scenario.behind_nginx else None,
            "env": {name: os.environ[name] for name in SANITIZER_OPTIONS if name in os.environ},
        }, scenario.netem)
    finally:
        slots.put((origin_cpus, client_cpus, slot))
    outcome = outcome_of(code, out)
    good = outcome != "crash" and scenario.expect in (outcome, "any")
    if good:
        stamp.write_text(key)
    else:
        stamp.unlink(missing_ok=True)
    return scenario, outcome if good else f"UNEXPECTED {outcome}", time.monotonic() - began, why(out, code)


def why(out, code):
    try:
        return json.load(open(out))["first_failures"][:3]
    except (OSError, ValueError, KeyError):
        tail = open(out + ".log", errors="replace").read()[-300:] if pathlib.Path(out + ".log").exists() else ""
        return [f"exit {code}: {tail}"]


def plan_slots(tier, origin_cpus, client_cpus):
    slots = queue.Queue()
    if tier == "full":
        everything = sorted(os.sched_getaffinity(0))
        middle = len(everything) // 2
        slots.put((origin_cpus or everything[:middle], client_cpus or everything[middle:], 0))
        return slots, 1
    everything = sorted(os.sched_getaffinity(0))
    count = max(1, len(everything) // SLOT_CPUS)
    for slot in range(count):
        part = everything[slot * SLOT_CPUS:(slot + 1) * SLOT_CPUS] or everything
        half = max(1, len(part) // 2)
        slots.put((part[:half], part[half:] or part, slot))
    return slots, count


def cpu_list(text):
    cpus = []
    for part in text.split(","):
        first, _, last = part.partition("-")
        cpus += range(int(first), int(last or first) + 1)
    return cpus


def build(profile):
    began = time.monotonic()
    env = {**os.environ, "CARGO_TERM_QUIET": "true"}
    code = subprocess.run(["cargo", "build", "--profile", profile, "--bin", "qualify"], cwd=HERE, env=env).returncode
    if code:
        sys.exit(code)
    print(f"build {time.monotonic() - began:.1f}s", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tier", choices=TIERS, default="quick")
    parser.add_argument("--only", help="regular expression over scenario names")
    parser.add_argument("--force", action="store_true", help="run scenarios that already passed on this binary")
    parser.add_argument("--no-build", action="store_true")
    parser.add_argument("--list", action="store_true")
    parser.add_argument("--origin-cpus", type=cpu_list, help="full tier: CPUs for the origin")
    parser.add_argument("--client-cpus", type=cpu_list, help="full tier: CPUs for the client")
    parser.add_argument("--binary", help="run this binary instead of building one")
    parser.add_argument("--preload", help="allocator library to load into the client")
    parser.add_argument("--nginx", default=shutil.which("nginx"), help="nginx binary for the scenarios behind nginx")
    parser.add_argument("--job", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.job:
        sys.exit(run_job(json.loads(args.job)))
    began = time.monotonic()
    plans = [p for p in TIERS[args.tier]() if (not args.only or re.search(args.only, p.name)) and (args.nginx or not p.behind_nginx)]
    if args.list:
        print("\n".join(p.name for p in plans))
        return 0
    profile = PROFILES[args.tier]
    if not (args.no_build or args.binary):
        build(profile)
    binary = args.binary or str(HERE / "target" / profile / "qualify")
    digest = hashlib.sha256(pathlib.Path(binary).read_bytes()).hexdigest()
    folder = HERE / "results" / args.tier
    folder.mkdir(parents=True, exist_ok=True)
    slots, width = plan_slots(args.tier, args.origin_cpus, args.client_cpus)
    keys = {p.name: hashlib.sha256((digest + str(args.preload) + repr(dataclasses.astuple(p))).encode()).hexdigest() for p in plans}
    if args.force:
        for plan in plans:
            (folder / f"{plan.name}.stamp").unlink(missing_ok=True)
    bad, tally = 0, {}
    with concurrent.futures.ThreadPoolExecutor(width) as pool:
        running = [pool.submit(execute, p, binary, keys[p.name], folder, slots, args.preload, args.nginx) for p in plans]
        for finished in concurrent.futures.as_completed(running):
            scenario, status, seconds, reasons = finished.result()
            tally[status] = tally.get(status, 0) + 1
            if status != "cached":
                print(f"{status:8} {seconds:6.1f}s {scenario.name}", flush=True)
            if status.startswith("UNEXPECTED"):
                bad += 1
                for reason in reasons:
                    print(f"         {reason[:200]}", flush=True)
    print(f"{args.tier}: {len(plans)} scenarios on {width} slots, {tally}, {time.monotonic() - began:.1f}s")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
