import asyncio

import httpx

from harness import EXPECTED, H1, POOL, headers, main, ssl_context

VERSION = "HTTP/1.1" if H1 else "HTTP/2"
HEADERS = headers()
VERIFY = ssl_context()


def build():
    if H1:
        limits = httpx.Limits(max_connections=POOL, max_keepalive_connections=POOL)
    else:
        limits = httpx.Limits(max_connections=1)
    return httpx.AsyncClient(http2=not H1, verify=VERIFY, limits=limits, headers=HEADERS, trust_env=False)


async def get(client, url):
    resp = await client.get(url)
    assert resp.status_code == 200, f"status {resp.status_code}"
    assert resp.http_version == VERSION, f"version {resp.http_version}"
    assert resp.content == EXPECTED, "response body mismatch"
    return resp.status_code, resp.content


asyncio.run(main(name="httpx", build=build, get=get, close=httpx.AsyncClient.aclose))
