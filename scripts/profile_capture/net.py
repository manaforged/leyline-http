from __future__ import annotations

import json
import shutil
import urllib.request
from pathlib import Path

def http_json(url: str, timeout: int = 30) -> dict:
    with urllib.request.urlopen(url, timeout=timeout) as resp:
        return json.loads(resp.read())


def http_bytes(url: str, dest: Path, timeout: int = 300) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    with urllib.request.urlopen(url, timeout=timeout) as resp, dest.open("wb") as out:
        shutil.copyfileobj(resp, out)
