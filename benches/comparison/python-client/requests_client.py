import math
import os
import ssl
import sys
import tempfile
import time
from concurrent.futures import ThreadPoolExecutor

import requests
import urllib3
from requests.adapters import HTTPAdapter

NAME = "requests"

if os.environ.get("CMP_PROTO", "h2") != "h1":
    sys.exit("requests speaks HTTP/1.1 only; set CMP_PROTO=h1")


def tls():
    ca = os.environ.get("CMP_CA")
    if ca is None:
        urllib3.disable_warnings(urllib3.exceptions.InsecureRequestWarning)
        return False
    with open(ca, "rb") as f:
        pem = ssl.DER_cert_to_PEM_cert(f.read())
    out = tempfile.NamedTemporaryFile("w", suffix=".pem", delete=False)
    out.write(pem)
    out.close()
    return out.name


VERIFY = tls()
POOL = max(64, int(sys.argv[5]) if len(sys.argv) > 5 else 64)


def build():
    session = requests.Session()
    session.verify = VERIFY
    session.mount("https://", HTTPAdapter(pool_connections=1, pool_maxsize=POOL, max_retries=0))
    return session


def expected_body():
    size = os.environ.get("CMP_BODY")
    if size is None:
        return b"ok-10byte!"
    return bytes(i % 251 for i in range(int(size)))


EXPECTED = expected_body()


def get(session, url):
    resp = session.get(url)
    assert resp.status_code == 200, f"status {resp.status_code}"
    assert resp.raw.version == 11, f"version {resp.raw.version}"
    assert resp.content == EXPECTED, "response body mismatch"


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


def main():
    url = sys.argv[1] if len(sys.argv) > 1 else "https://127.0.0.1:8443/"

    if len(sys.argv) > 2 and sys.argv[2] == "equiv":
        with build() as session:
            resp = session.get(url)
            body = resp.content
            print(f"EQUIV {NAME} status={resp.status_code} fnv={fnv1a(body):016x} len={len(body)}")
        return

    warm_n = arg(2, 2000)
    cold_n = arg(3, 200)
    conc_n = arg(4, 20000)
    conc_c = arg(5, 64)
    connections = max(1, int(os.environ.get("CMP_CONNECTIONS", "1")))
    sessions = [build() for _ in range(connections)]
    for s in sessions:
        get(s, url)

    session = sessions[0]
    warm = []
    start = time.perf_counter()
    for _ in range(warm_n):
        t0 = time.perf_counter_ns()
        get(session, url)
        warm.append((time.perf_counter_ns() - t0) // 1000)
    warm_rps = warm_n / (time.perf_counter() - start)
    warm.sort()

    per = conc_n // conc_c

    def worker(s):
        lat = []
        for _ in range(per):
            t0 = time.perf_counter_ns()
            get(s, url)
            lat.append((time.perf_counter_ns() - t0) // 1000)
        return lat

    start = time.perf_counter()
    with ThreadPoolExecutor(conc_c) as pool:
        results = list(pool.map(worker, (sessions[i % connections] for i in range(conc_c))))
    conc_rps = per * conc_c / (time.perf_counter() - start)
    conc = sorted(v for lat in results for v in lat)

    start = time.perf_counter()
    for _ in range(cold_n):
        with build() as fresh:
            get(fresh, url)
    cold_rps = cold_n / (time.perf_counter() - start)

    for s in sessions:
        s.close()

    print(
        f"RESULT {NAME} connections={connections} warm_rps={warm_rps:.3f} conc_rps={conc_rps:.3f} cold_rps={cold_rps:.3f}"
        f" warm_p50_us={pct(warm, 0.50)} warm_p90_us={pct(warm, 0.90)} warm_p99_us={pct(warm, 0.99)}"
        f" conc_p50_us={pct(conc, 0.50)} conc_p90_us={pct(conc, 0.90)} conc_p99_us={pct(conc, 0.99)} conc_p999_us={pct(conc, 0.999)}"
    )


main()
