import asyncio
import os
import sys

import leyline

from harness import EXPECTED, H1, PROTO, ca_path, headers, main

VERSION = "HTTP/1.1" if H1 else "HTTP/2"

if len(sys.argv) > 1 and sys.argv[1] == "profiles":
    print("\n".join(p.removeprefix("chrome-") for p in leyline.profiles() if p.startswith("chrome-")))
    sys.exit(0)

chrome = os.environ.get("CMP_CHROME")
if chrome and not chrome.isdecimal():
    sys.exit(f"leyline-python: CMP_CHROME must be a version number, got {chrome}")
if os.environ.get("CMP_TIMEOUT", "default") != "default":
    sys.exit("leyline-python supports only CMP_TIMEOUT=default")
BROWSER = f"chrome{chrome}" if chrome else "none"
CA = ca_path()
HEADERS = headers()


def build():
    return leyline.AsyncSession(
        browser=BROWSER,
        protocol=PROTO,
        trust_env=False,
        verify=bool(CA),
        ca_files=[CA] if CA else None,
        headers=HEADERS,
    )


async def get(client, url):
    response = await client.get(url)
    body = response.bytes()
    assert response.status == 200, f"status {response.status}"
    assert response.http_version == VERSION, f"version {response.http_version}"
    assert body == EXPECTED, "response body mismatch"
    return response.status, body


asyncio.run(main(name="leyline-python", build=build, get=get, close=leyline.AsyncSession.close))
