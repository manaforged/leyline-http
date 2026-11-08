import os
import sys
import time
from concurrent.futures import ThreadPoolExecutor

if os.environ.get("CMP_PROTO", "h2") != "h1":
    sys.exit("requests speaks HTTP/1.1 only; set CMP_PROTO=h1")

import requests
import urllib3
from requests.adapters import HTTPAdapter

from harness import EXPECTED, POOL, arg, ca_path, connections, headers, is_equiv, print_equiv, report, url

NAME = "requests"
VERIFY = ca_path() or False
HEADERS = headers()
if not VERIFY:
    urllib3.disable_warnings(urllib3.exceptions.InsecureRequestWarning)


def build():
    session = requests.Session()
    session.verify = VERIFY
    session.trust_env = False
    if HEADERS:
        session.headers.clear()
        session.headers.update(HEADERS)
    session.mount("https://", HTTPAdapter(pool_connections=1, pool_maxsize=POOL, max_retries=0))
    return session


def get(session, target):
    resp = session.get(target)
    assert resp.status_code == 200, f"status {resp.status_code}"
    assert resp.raw.version == 11, f"version {resp.raw.version}"
    assert resp.content == EXPECTED, "response body mismatch"


def main():
    target = url()
    if is_equiv():
        with build() as session:
            resp = session.get(target)
            print_equiv(NAME, resp.status_code, resp.content)
        return

    warm_n = arg(2, 2000)
    cold_n = arg(3, 200)
    conc_n = arg(4, 20000)
    conc_c = arg(5, 64)
    conns = connections()
    sessions = [build() for _ in range(conns)]
    for s in sessions:
        get(s, target)

    warm = []
    start = time.perf_counter()
    for _ in range(warm_n):
        t0 = time.perf_counter_ns()
        get(sessions[0], target)
        warm.append((time.perf_counter_ns() - t0) // 1000)
    warm_rps = warm_n / (time.perf_counter() - start)
    warm.sort()

    per = conc_n // conc_c

    def worker(s):
        lat = []
        for _ in range(per):
            t0 = time.perf_counter_ns()
            get(s, target)
            lat.append((time.perf_counter_ns() - t0) // 1000)
        return lat

    start = time.perf_counter()
    with ThreadPoolExecutor(conc_c) as pool:
        results = list(pool.map(worker, (sessions[i % conns] for i in range(conc_c))))
    conc_rps = per * conc_c / (time.perf_counter() - start)
    conc = sorted(v for lat in results for v in lat)

    start = time.perf_counter()
    for _ in range(cold_n):
        with build() as fresh:
            get(fresh, target)
    cold_rps = cold_n / (time.perf_counter() - start)

    for s in sessions:
        s.close()

    report(NAME, conns, warm_rps, conc_rps, cold_rps, warm, conc)


main()
