from __future__ import annotations

import json
import os
import subprocess


def notify(title: str, body: str) -> None:
    print(f"::warning::{title}")
    if not os.environ.get("GH_REPO"):
        return
    found = subprocess.run(
        ["gh", "issue", "list", "--state", "open", "--search", f'"{title}" in:title', "--json", "title"],
        capture_output=True, text=True, check=True,
    )
    if any(issue["title"] == title for issue in json.loads(found.stdout)):
        return
    command = ["gh", "issue", "create", "--title", title, "--body", body]
    assignee = os.environ.get("NOTIFY")
    if assignee:
        command += ["--assignee", assignee]
    subprocess.run(command, check=True)
