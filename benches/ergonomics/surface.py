import importlib.util
import json
import subprocess
import sys
import tomllib
from pathlib import Path

import score

ROOT = Path(__file__).resolve().parent
OUT = ROOT / "work" / "surface"
RUST_CRATES = {"futures": ["futures", "futures-util", "futures-core"]}
SHARED_RUST = ["http"]


def rust(lib):
    project = ROOT / "rust" / lib["name"]
    meta = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--format-version", "1"],
            cwd=project,
            capture_output=True,
            text=True,
            check=True,
        ).stdout
    )
    by_name = {}
    for package in meta["packages"]:
        by_name.setdefault(package["name"], []).append(Path(package["manifest_path"]).parent)
    declared = tomllib.loads((project / "Cargo.toml").read_text())["dependencies"]
    result = {}
    for dep in declared:
        names = RUST_CRATES.get(dep, [dep]) + SHARED_RUST
        files = []
        for name in names:
            for directory in by_name.get(name, []):
                files.extend((directory / "src").rglob("*.rs"))
        result[dep] = sorted(score.extract_surface("rust", files))
    return result


def python(lib):
    result = {}
    for path in sorted((ROOT / "python" / lib["name"]).glob("t*.py")):
        _, roots, _, _ = score.python_analyze(score.tokens(path.read_text(), "python"), lib["name"], None, path.read_text())
        for root in roots:
            if root in sys.stdlib_module_names or root in result:
                continue
            spec = importlib.util.find_spec(root)
            if spec is None or spec.origin is None:
                continue
            origin = Path(spec.origin)
            files = list(origin.parent.rglob("*.py")) + list(origin.parent.rglob("*.pyi")) if origin.name.startswith("__init__") else [origin]
            files = [f for f in files if "tests" not in f.parts]
            result[root] = sorted(score.extract_surface("python", files))
    return result


def node(lib):
    modules = ROOT / "node" / "node_modules"
    result = {}
    packages = json.loads((ROOT / "node" / "package.json").read_text())["dependencies"]
    for name in [*packages, "leyline-http"]:
        base = (modules / name).resolve()
        files = [f for pattern in ("*.d.ts", "*.d.mts", "*.d.cts") for f in base.rglob(pattern) if "node_modules" not in f.relative_to(base).parts]
        result[name] = sorted(score.extract_surface("node", files))
    types = modules / "undici-types"
    result["fetch"] = sorted(score.extract_surface("node", [types / f for f in ("fetch.d.ts", "formdata.d.ts", "file.d.ts")]))
    return result


def main():
    for lib in tomllib.loads((ROOT / "libraries.toml").read_text())["library"]:
        extract = {"rust": rust, "python": python, "node": node}[lib["lang"]]
        target = OUT / lib["lang"] / f"{lib['name']}.json"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(json.dumps(extract(lib)))


if __name__ == "__main__":
    main()
