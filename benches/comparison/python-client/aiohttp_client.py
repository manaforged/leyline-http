import asyncio
import math
import os
import ssl
import sys
import time

import aiohttp

NAME = "aiohttp"

if os.environ.get("CMP_PROTO", "h2") != "h1":
    sys.exit("aiohttp speaks HTTP/1.1 only; set CMP_PROTO=h1")


def tls():
    ca = os.environ.get("CMP_CA")
    if ca is None:
        return False
    with open(ca, "rb") as f:
        return ssl.create_default_context(cadata=f.read())


SSL = tls()
POOL = max(64, int(sys.argv[5]) if len(sys.argv) > 5 else 64)


def build():
    return aiohttp.ClientSession(connector=aiohttp.TCPConnector(limit=POOL, limit_per_host=POOL, ssl=SSL))


def expected_body():
    size = os.environ.get("CMP_BODY")
    if size is None:
        return b"ok-10byte!"
    return bytes(i % 251 for i in range(int(size)))


EXPECTED = expected_body()


async def get(session, url):
    async with session.get(url) as resp:
        body = await resp.read()
        assert resp.status == 200, f"status {resp.status}"
        assert resp.version == aiohttp.HttpVersion11, f"version {resp.version}"
        assert body == EXPECTED, "response body mismatch"


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


async def main():
    url = sys.argv[1] if len(sys.argv) > 1 else "https://127.0.0.1:8443/"

    if len(sys.argv) > 2 and sys.argv[2] == "equiv":
        async with build() as session:
            async with session.get(url) as resp:
                body = await resp.read()
                print(f"EQUIV {NAME} status={resp.status} fnv={fnv1a(body):016x} len={len(body)}")
        return

    warm_n = arg(2, 2000)
    cold_n = arg(3, 200)
    conc_n = arg(4, 20000)
    conc_c = arg(5, 64)
    connections = max(1, int(os.environ.get("CMP_CONNECTIONS", "1")))
    sessions = [build() for _ in range(connections)]
    for s in sessions:
        await get(s, url)

    session = sessions[0]
    warm = []
    start = time.perf_counter()
    for _ in range(warm_n):
        t0 = time.perf_counter_ns()
        await get(session, url)
        warm.append((time.perf_counter_ns() - t0) // 1000)
    warm_rps = warm_n / (time.perf_counter() - start)
    warm.sort()

    per = conc_n // conc_c

    async def worker(s):
        lat = []
        for _ in range(per):
            t0 = time.perf_counter_ns()
            await get(s, url)
            lat.append((time.perf_counter_ns() - t0) // 1000)
        return lat

    start = time.perf_counter()
    results = await asyncio.gather(*(asyncio.create_task(worker(sessions[i % connections])) for i in range(conc_c)))
    conc_rps = per * conc_c / (time.perf_counter() - start)
    conc = sorted(v for lat in results for v in lat)

    start = time.perf_counter()
    for _ in range(cold_n):
        async with build() as fresh:
            await get(fresh, url)
    cold_rps = cold_n / (time.perf_counter() - start)

    for s in sessions:
        await s.close()

    print(
        f"RESULT {NAME} connections={connections} warm_rps={warm_rps:.3f} conc_rps={conc_rps:.3f} cold_rps={cold_rps:.3f}"
        f" warm_p50_us={pct(warm, 0.50)} warm_p90_us={pct(warm, 0.90)} warm_p99_us={pct(warm, 0.99)}"
        f" conc_p50_us={pct(conc, 0.50)} conc_p90_us={pct(conc, 0.90)} conc_p99_us={pct(conc, 0.99)} conc_p999_us={pct(conc, 0.999)}"
    )


asyncio.run(main())
