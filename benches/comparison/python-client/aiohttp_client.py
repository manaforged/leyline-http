import asyncio
import os
import sys

if os.environ.get("CMP_PROTO", "h2") != "h1":
    sys.exit("aiohttp speaks HTTP/1.1 only; set CMP_PROTO=h1")

import aiohttp

from harness import EXPECTED, POOL, headers, main, ssl_context

HEADERS = headers()
SSL = ssl_context()


def build():
    connector = aiohttp.TCPConnector(limit=POOL, limit_per_host=POOL, ssl=SSL)
    return aiohttp.ClientSession(connector=connector, headers=HEADERS)


async def get(session, url):
    async with session.get(url) as resp:
        body = await resp.read()
        assert resp.status == 200, f"status {resp.status}"
        assert resp.version == aiohttp.HttpVersion11, f"version {resp.version}"
        assert body == EXPECTED, "response body mismatch"
        return resp.status, body


asyncio.run(main(name="aiohttp", build=build, get=get, close=aiohttp.ClientSession.close))
