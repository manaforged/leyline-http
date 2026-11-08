import threading
import time

import pycurl

from harness import EXPECTED, H1, arg, ca_path, connections, headers, is_equiv, print_equiv, report, url

NAME = "pycurl"
REQUEST_VERSION = pycurl.CURL_HTTP_VERSION_1_1 if H1 else pycurl.CURL_HTTP_VERSION_2TLS
WIRE_VERSION = pycurl.CURL_HTTP_VERSION_1_1 if H1 else pycurl.CURL_HTTP_VERSION_2_0
HEADERS = [f"{k}: {v}" for k, v in (headers() or {}).items()]
CA = ca_path()


def handle(target, left, lat):
    c = pycurl.Curl()
    c.setopt(pycurl.URL, target)
    c.setopt(pycurl.HTTP_VERSION, REQUEST_VERSION)
    c.setopt(pycurl.HTTPHEADER, HEADERS)
    c.setopt(pycurl.NOSIGNAL, 1)
    c.setopt(pycurl.PROXY, "")
    if CA:
        c.setopt(pycurl.CAINFO, CA)
    else:
        c.setopt(pycurl.SSL_VERIFYPEER, 0)
        c.setopt(pycurl.SSL_VERIFYHOST, 0)
    c.chunks = []
    c.setopt(pycurl.WRITEFUNCTION, c.chunks.append)
    c.left = left
    c.lat = lat
    return c


def start(multi, c):
    c.chunks.clear()
    c.t0 = time.perf_counter_ns()
    multi.add_handle(c)


def finish(c):
    status = c.getinfo(pycurl.RESPONSE_CODE)
    version = c.getinfo(pycurl.INFO_HTTP_VERSION)
    body = b"".join(c.chunks)
    c.lat.append((time.perf_counter_ns() - c.t0) // 1000)
    assert status == 200, f"status {status}"
    assert version == WIRE_VERSION, f"version {version}"
    assert body == EXPECTED, "response body mismatch"
    return status, body


def drive(multi, handles):
    active = len(handles)
    for c in handles:
        start(multi, c)
    while active:
        while multi.perform()[0] == pycurl.E_CALL_MULTI_PERFORM:
            pass
        restarted = False
        while True:
            queued, done, failed = multi.info_read()
            if failed:
                raise RuntimeError(f"curl error {failed[0][1]}: {failed[0][2]}")
            for c in done:
                multi.remove_handle(c)
                finish(c)
                c.left -= 1
                if c.left:
                    start(multi, c)
                    restarted = True
                else:
                    active -= 1
            if not queued:
                break
        if active and not restarted:
            wait = multi.timeout()
            if wait != 0:
                multi.select(wait / 1000 if wait > 0 else 1.0)


def one(multi, target, lat):
    drive(multi, [handle(target, 1, lat)])


def main():
    target = url()
    if is_equiv():
        c = handle(target, 1, [])
        drive(pycurl.CurlMulti(), [c])
        print_equiv(NAME, c.getinfo(pycurl.RESPONSE_CODE), b"".join(c.chunks))
        return

    warm_n = arg(2, 2000)
    cold_n = arg(3, 200)
    conc_n = arg(4, 20000)
    conc_c = arg(5, 64)
    conns = connections()
    multis = [pycurl.CurlMulti() for _ in range(conns)]
    for m in multis:
        one(m, target, [])

    warm = []
    start_at = time.perf_counter()
    drive(multis[0], [handle(target, warm_n, warm)])
    warm_rps = warm_n / (time.perf_counter() - start_at)
    warm.sort()

    per = conc_n // conc_c
    lats = [[] for _ in range(conc_c)]
    groups = [[] for _ in range(conns)]
    for i in range(conc_c):
        groups[i % conns].append(handle(target, per, lats[i]))
    threads = [threading.Thread(target=drive, args=(m, g)) for m, g in zip(multis, groups) if g]
    start_at = time.perf_counter()
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    conc_rps = per * conc_c / (time.perf_counter() - start_at)
    conc = sorted(v for lat in lats for v in lat)
    assert len(conc) == per * conc_c, "a driver thread failed"

    start_at = time.perf_counter()
    for _ in range(cold_n):
        one(pycurl.CurlMulti(), target, [])
    cold_rps = cold_n / (time.perf_counter() - start_at)

    report(NAME, conns, warm_rps, conc_rps, cold_rps, warm, conc)


main()
