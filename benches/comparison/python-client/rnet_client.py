import asyncio
from pathlib import Path

import rnet

from harness import EXPECTED, H1, POOL, ca_path, headers, main

VERSION = rnet.Version.HTTP_11 if H1 else rnet.Version.HTTP_2
HEADERS = headers()
CA = ca_path()


def build():
    return rnet.Client(
        default_headers=HEADERS,
        http1_only=H1,
        http2_only=not H1,
        verify=Path(CA) if CA else False,
        pool_max_idle_per_host=POOL,
        pool_max_size=POOL,
        no_proxy=True,
    )


async def get(client, url):
    resp = await client.get(url)
    body = await resp.bytes()
    assert resp.status == 200, f"status {resp.status}"
    assert resp.version == VERSION, f"version {resp.version}"
    assert body == EXPECTED, "response body mismatch"
    return resp.status, body


async def close(client):
    pass


asyncio.run(main(name="rnet", build=build, get=get, close=close))
