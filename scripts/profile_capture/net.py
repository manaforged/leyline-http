from __future__ import annotations

import json
import os
import shutil
import urllib.request
from pathlib import Path

from .config import GITHUB_TOKEN_ENV, WAITS


def open_url(url: str, timeout: float, headers: dict[str, str] | None = None):
    return urllib.request.urlopen(urllib.request.Request(url, headers=headers or {}), timeout=timeout)


def http_text(url: str, timeout: float = WAITS["metadata_fetch"], headers: dict[str, str] | None = None) -> str:
    with open_url(url, timeout, headers) as resp:
        return resp.read().decode()


def http_json(url: str, timeout: float = WAITS["metadata_fetch"], headers: dict[str, str] | None = None) -> dict:
    return json.loads(http_text(url, timeout, headers))


def http_bytes(url: str, dest: Path, timeout: float = WAITS["download"]) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    with open_url(url, timeout) as resp, dest.open("wb") as out:
        shutil.copyfileobj(resp, out)


def github_json(url: str) -> dict:
    token = os.environ.get(GITHUB_TOKEN_ENV)
    return http_json(url, headers={"Authorization": f"Bearer {token}"} if token else None)
