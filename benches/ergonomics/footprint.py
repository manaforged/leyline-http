import json
import os
import shutil
import statistics
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent
WORK = ROOT / "work" / "footprint"
OUT = ROOT / "footprint.json"
RUST = ["leyline-http", "reqwest", "ureq", "wreq"]
PYTHON = {"requests": "requests==2.34.2", "httpx": "httpx==0.28.1", "aiohttp": "aiohttp==3.14.5", "curl_cffi": "curl_cffi==0.17.0", "niquests": "niquests==3.21.2"}
NODE = {"axios": "axios@1.21.0", "got": "got@16.2.0", "undici": "undici@8.12.4"}
ROUNDS = 3


def tree_bytes(path, skip=()):
    total = 0
    for root, dirs, files in os.walk(path):
        dirs[:] = [d for d in dirs if d not in skip]
        for name in files:
            file = Path(root) / name
            if not file.is_symlink():
                total += file.stat().st_size
    return total


def save(data):
    OUT.write_text(json.dumps(data, indent=1, sort_keys=True) + "\n")


def rust(data):
    env = {**os.environ, "CARGO_BUILD_JOBS": "4"}
    for lib in RUST:
        subprocess.run(["cargo", "fetch"], cwd=ROOT / "rust" / "hello" / lib, check=True, env=env)
    for _ in range(ROUNDS):
        for lib in RUST:
            entry = data.setdefault(f"rust/{lib}", {})
            if len(entry.get("cold_build_seconds", [])) >= ROUNDS:
                continue
            target = WORK / "rust" / lib
            shutil.rmtree(target, ignore_errors=True)
            started = time.monotonic()
            built = subprocess.run(
                ["nice", "-n", "19", "cargo", "build", "--release", "--offline"],
                cwd=ROOT / "rust" / "hello" / lib,
                env={**env, "CARGO_TARGET_DIR": str(target)},
                check=True,
                capture_output=True,
            )
            entry.setdefault("cold_build_seconds", []).append(round(time.monotonic() - started, 1))
            binary = target / "release" / "hello"
            entry["binary_bytes"] = binary.stat().st_size
            stripped = target / "hello-stripped"
            subprocess.run(["strip", "-o", str(stripped), str(binary)], check=True)
            entry["stripped_bytes"] = stripped.stat().st_size
            entry["crates_compiled"] = built.stderr.decode().count("Compiling ")
            save(data)
            shutil.rmtree(target, ignore_errors=True)
            print(lib, entry["cold_build_seconds"], flush=True)


def python_sizes(data, base_python, leyline_dir):
    for lib, spec in PYTHON.items():
        if "installed_bytes" in data.get(f"python/{lib}", {}):
            continue
        venv = WORK / "py" / lib
        shutil.rmtree(venv, ignore_errors=True)
        subprocess.run([base_python, "-m", "venv", str(venv)], check=True)
        before = tree_bytes(venv)
        subprocess.run([str(venv / "bin" / "pip"), "install", "-q", spec], check=True)
        data.setdefault(f"python/{lib}", {})["installed_bytes"] = tree_bytes(venv) - before
        shutil.rmtree(venv, ignore_errors=True)
    data.setdefault("python/leyline", {})["installed_bytes"] = tree_bytes(leyline_dir, ("__pycache__",))
    save(data)


def node_sizes(data, leyline_dir):
    for lib, spec in NODE.items():
        if "installed_bytes" in data.get(f"node/{lib}", {}):
            continue
        directory = WORK / "node" / lib
        shutil.rmtree(directory, ignore_errors=True)
        directory.mkdir(parents=True)
        subprocess.run(["npm", "install", "--silent", "--no-audit", "--no-fund", spec], cwd=directory, check=True)
        data.setdefault(f"node/{lib}", {})["installed_bytes"] = tree_bytes(directory / "node_modules")
        shutil.rmtree(directory, ignore_errors=True)
    data.setdefault("node/fetch", {})["installed_bytes"] = 0
    wrapper = Path(leyline_dir)
    files = [f for f in wrapper.iterdir() if f.is_file() and f.suffix in (".node", ".js", ".ts")]
    data.setdefault("node/leyline-http", {})["installed_bytes"] = sum(f.stat().st_size for f in files)
    save(data)


def main():
    python_dir, node_dir = sys.argv[1], sys.argv[2]
    data = json.loads(OUT.read_text()) if OUT.exists() else {}
    python_sizes(data, sys.executable, python_dir)
    node_sizes(data, node_dir)
    rust(data)
    for key, entry in sorted(data.items()):
        runs = entry.get("cold_build_seconds")
        print(key, entry, "median", statistics.median(runs) if runs else None)


if __name__ == "__main__":
    main()
