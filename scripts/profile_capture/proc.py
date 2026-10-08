from __future__ import annotations

import os
import signal
import subprocess


def spawn(cmd: list[str], **kwargs: object) -> subprocess.Popen:
    return subprocess.Popen(cmd, start_new_session=True, **kwargs)


def stop_tree(proc: subprocess.Popen, timeout: float) -> None:
    if proc.poll() is None:
        if os.name == "nt":
            subprocess.run(
                ["taskkill", "/T", "/F", "/PID", str(proc.pid)],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                check=False,
            )
        else:
            try:
                os.killpg(proc.pid, signal.SIGKILL)
            except OSError:
                pass
    proc.wait(timeout=timeout)
