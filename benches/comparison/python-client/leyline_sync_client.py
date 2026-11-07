import os
import sys
import time
from concurrent.futures import ThreadPoolExecutor

import leyline

from harness import EXPECTED, H1, PROTO, arg, ca_path, connections, headers, is_equiv, print_equiv, report, url

NAME = "leyline-python-sync"
VERSION = "HTTP/1.1" if H1 else "HTTP/2"
CA = ca_path()
HEADERS = headers()
if os.environ.get("CMP_TIMEOUT", "default") != "default":
    sys.exit("leyline-python-sync supports only CMP_TIMEOUT=default")


def build():
    return leyline.Session(
        browser="none",
        protocol=PROTO,
        trust_env=False,
        verify=bool(CA),
        ca_files=[CA] if CA else None,
        headers=HEADERS,
    )


def get(session, target):
    response = session.get(target)
    assert response.status == 200, f"status {response.status}"
    assert response.http_version == VERSION, f"version {response.http_version}"
    assert response.bytes() == EXPECTED, "response body mismatch"


def main():
    target = url()
    session = build()
    if is_equiv():
        response = session.get(target)
        print_equiv(NAME, response.status, response.bytes())
        return

    warm_n = arg(2, 2000)
    cold_n = arg(3, 200)
    conc_n = arg(4, 20000)
    conc_c = arg(5, 64)
    get(session, target)

    warm = []
    start = time.perf_counter()
    for _ in range(warm_n):
        t0 = time.perf_counter_ns()
        get(session, target)
        warm.append((time.perf_counter_ns() - t0) // 1000)
    warm_rps = warm_n / (time.perf_counter() - start)
    warm.sort()

    per = conc_n // conc_c

    def worker(_):
        lat = []
        for _ in range(per):
            t0 = time.perf_counter_ns()
            get(session, target)
            lat.append((time.perf_counter_ns() - t0) // 1000)
        return lat

    start = time.perf_counter()
    with ThreadPoolExecutor(conc_c) as pool:
        results = list(pool.map(worker, range(conc_c)))
    conc_rps = per * conc_c / (time.perf_counter() - start)
    conc = sorted(v for lat in results for v in lat)

    start = time.perf_counter()
    for _ in range(cold_n):
        get(build(), target)
    cold_rps = cold_n / (time.perf_counter() - start)

    report(NAME, connections(), warm_rps, conc_rps, cold_rps, warm, conc)


main()
