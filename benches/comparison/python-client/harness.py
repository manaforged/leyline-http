import hashlib
import math
import os
import ssl
import sys
import tempfile
import time

PROTO = os.environ.get("CMP_PROTO", "h2")
if PROTO not in ("h1", "h2"):
    sys.exit(f"CMP_PROTO must be h1 or h2, got {PROTO}")
H1 = PROTO == "h1"
POOL = max(64, int(sys.argv[5]) if len(sys.argv) > 5 else 64)


def headers():
    path = os.environ.get("CMP_HEADERS")
    if not path:
        return None
    with open(path) as f:
        return dict(line.rstrip("\n").split("\t", 1) for line in f if line.strip())


def ca_path():
    ca = os.environ.get("CMP_CA")
    if ca is None:
        return None
    with open(ca, "rb") as f:
        data = f.read()
    if data.startswith(b"-----BEGIN"):
        return ca
    path = os.path.join(tempfile.gettempdir(), f"cmp-ca-{hashlib.sha256(data).hexdigest()[:16]}.pem")
    with open(path, "w") as f:
        f.write(ssl.DER_cert_to_PEM_cert(data))
    return path


def ssl_context():
    ca = ca_path()
    return ssl.create_default_context(cafile=ca) if ca else False


def expected_body():
    size = os.environ.get("CMP_BODY")
    if size is None:
        return b"ok-10byte!"
    return bytes(i % 251 for i in range(int(size)))


EXPECTED = expected_body()


def fnv1a(data):
    h = 0xCBF29CE484222325
    for b in data:
        h ^= b
        h = (h * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h


def pct(values, p):
    return values[math.floor((len(values) - 1) * p + 0.5)]


def arg(i, default):
    return int(sys.argv[i]) if len(sys.argv) > i else default


def url():
    return sys.argv[1] if len(sys.argv) > 1 else "https://127.0.0.1:8443/"


def is_equiv():
    return len(sys.argv) > 2 and sys.argv[2] == "equiv"


def print_equiv(name, status, body):
    print(f"EQUIV {name} status={status} fnv={fnv1a(body):016x} len={len(body)}")


def connections():
    return max(1, int(os.environ.get("CMP_CONNECTIONS", "1")))


def report(name, conns, warm_rps, conc_rps, cold_rps, warm, conc):
    print(
        f"RESULT {name} connections={conns} warm_rps={warm_rps:.3f} conc_rps={conc_rps:.3f} cold_rps={cold_rps:.3f}"
        f" warm_p50_us={pct(warm, 0.50)} warm_p90_us={pct(warm, 0.90)} warm_p99_us={pct(warm, 0.99)}"
        f" conc_p50_us={pct(conc, 0.50)} conc_p90_us={pct(conc, 0.90)} conc_p99_us={pct(conc, 0.99)} conc_p999_us={pct(conc, 0.999)}"
    )


async def main(*, name, build, get, close):
    import asyncio

    target = url()
    if is_equiv():
        client = build()
        status, body = await get(client, target)
        print_equiv(name, status, body)
        await close(client)
        return

    warm_n = arg(2, 2000)
    cold_n = arg(3, 200)
    conc_n = arg(4, 20000)
    conc_c = arg(5, 64)
    conns = connections()
    clients = [build() for _ in range(conns)]
    for c in clients:
        await get(c, target)

    warm = []
    start = time.perf_counter()
    for _ in range(warm_n):
        t0 = time.perf_counter_ns()
        await get(clients[0], target)
        warm.append((time.perf_counter_ns() - t0) // 1000)
    warm_rps = warm_n / (time.perf_counter() - start)
    warm.sort()

    per = conc_n // conc_c

    async def worker(c):
        lat = []
        for _ in range(per):
            t0 = time.perf_counter_ns()
            await get(c, target)
            lat.append((time.perf_counter_ns() - t0) // 1000)
        return lat

    start = time.perf_counter()
    results = await asyncio.gather(*(asyncio.create_task(worker(clients[i % conns])) for i in range(conc_c)))
    conc_rps = per * conc_c / (time.perf_counter() - start)
    conc = sorted(v for lat in results for v in lat)

    start = time.perf_counter()
    for _ in range(cold_n):
        fresh = build()
        await get(fresh, target)
        await close(fresh)
    cold_rps = cold_n / (time.perf_counter() - start)

    for c in clients:
        await close(c)

    report(name, conns, warm_rps, conc_rps, cold_rps, warm, conc)
