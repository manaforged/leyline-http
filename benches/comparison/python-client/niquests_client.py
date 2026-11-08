import asyncio

import niquests

from harness import EXPECTED, H1, POOL, ca_path, headers, main

VERSION = 11 if H1 else 20
HEADERS = headers()
VERIFY = ca_path() or False


def build():
    session = niquests.AsyncSession(
        multiplexed=not H1,
        disable_http2=H1,
        disable_http3=True,
        pool_connections=1,
        pool_maxsize=POOL if H1 else 1,
        retries=0,
        headers=HEADERS,
        verify=VERIFY,
        revocation_configuration=None,
    )
    session.trust_env = False
    return session


async def get(session, url):
    resp = await session.get(url)
    if not H1:
        await session.gather(resp)
    assert resp.status_code == 200, f"status {resp.status_code}"
    assert resp.raw.version == VERSION, f"version {resp.raw.version}"
    assert resp.content == EXPECTED, "response body mismatch"
    return resp.status_code, resp.content


asyncio.run(main(name="niquests", build=build, get=get, close=niquests.AsyncSession.close))
