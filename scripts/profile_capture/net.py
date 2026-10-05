from __future__ import annotations

import json
import shutil
import urllib.request
from pathlib import Path

from .config import WAITS


def http_json(url: str, timeout: float = WAITS["metadata_fetch"]) -> dict:
    with urllib.request.urlopen(url, timeout=timeout) as resp:
        return json.loads(resp.read())


def http_bytes(url: str, dest: Path, timeout: float = WAITS["download"]) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    with urllib.request.urlopen(url, timeout=timeout) as resp, dest.open("wb") as out:
        shutil.copyfileobj(resp, out)
