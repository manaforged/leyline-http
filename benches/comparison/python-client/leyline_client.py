import asyncio
import os
import sys
import ssl
import tempfile
from pathlib import Path

import leyline

from httpx_client import EXPECTED, VERSION, main

if len(sys.argv) > 1 and sys.argv[1] == "profiles":
    print("\n".join(p.removeprefix("chrome-") for p in leyline.profiles() if p.startswith("chrome-")))
    sys.exit(0)

chrome = os.environ.get("CMP_CHROME")
if not chrome or not chrome.isdecimal():
    sys.exit("leyline-python requires an explicit CMP_CHROME browser profile")
if os.environ.get("CMP_TIMEOUT", "default") != "default":
    sys.exit("leyline-python supports only CMP_TIMEOUT=default")
protocol = os.environ.get("CMP_PROTO", "h2")
if protocol not in ("h1", "h2"):
    sys.exit("leyline-python supports CMP_PROTO=h1 or h2")
ca = os.environ.get("CMP_CA")


def build():
    return leyline.AsyncSession(
        browser=f"chrome{chrome}",
        protocol=protocol,
        trust_env=False,
        verify=bool(ca),
        ca_files=[ca_path] if ca_path else None,
    )


async def get(client, url):
    response = await client.get(url)
    body = response.bytes()
    assert response.status == 200, f"status {response.status}"
    assert response.http_version == VERSION, f"version {response.http_version}"
    assert body == EXPECTED, "response body mismatch"
    return response.status, body


if ca:
    data = Path(ca).read_bytes()
    pem = data.decode("ascii") if data.startswith(b"-----BEGIN") else ssl.DER_cert_to_PEM_cert(data)
    with tempfile.NamedTemporaryFile(mode="w", suffix=".pem") as certificate:
        certificate.write(pem)
        certificate.flush()
        ca_path = certificate.name
        asyncio.run(main(name="leyline-python", build=build, get=get, close=leyline.AsyncSession.close))
else:
    ca_path = None
    asyncio.run(main(name="leyline-python", build=build, get=get, close=leyline.AsyncSession.close))
