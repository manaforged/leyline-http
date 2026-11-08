import asyncio

from curl_cffi import CurlHttpVersion
from curl_cffi.requests import AsyncSession

from harness import EXPECTED, H1, POOL, ca_path, headers, main

HTTP_VERSION = "v1" if H1 else "v2tls"
WIRE_VERSION = CurlHttpVersion.V1_1 if H1 else CurlHttpVersion.V2_0
HEADERS = headers()
VERIFY = ca_path() or False


def build():
    return AsyncSession(
        max_clients=POOL,
        http_version=HTTP_VERSION,
        verify=VERIFY,
        headers=HEADERS,
        trust_env=False,
    )


async def get(session, url):
    resp = await session.get(url)
    assert resp.status_code == 200, f"status {resp.status_code}"
    assert resp.http_version == WIRE_VERSION, f"version {resp.http_version}"
    assert resp.content == EXPECTED, "response body mismatch"
    return resp.status_code, resp.content


asyncio.run(main(name="curl-cffi", build=build, get=get, close=AsyncSession.close))
