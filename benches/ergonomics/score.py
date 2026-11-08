import ast
import json
import re
import sys
import tomllib
from pathlib import Path

from lexer import code_lines, tokens

ROOT = Path(__file__).resolve().parent

VERBS = {"get", "post", "put", "patch", "delete", "head", "request", "json", "text", "status", "headers", "body", "url"}

RUST_STD = set(
    """
    abs all any as_bytes as_deref as_mut as_ref as_slice as_str await by_ref chars clone collect contains
    count create default drop ends_with enumerate eq err expect extend filter filter_map find first flatten
    flush fold for_each format from from_str from_secs from_millis get_mut insert into into_iter is_empty
    is_err is_ok is_none is_some iter iter_mut join last len lines lock map map_err max min new next nth
    ok ok_or open parse pop position push push_str read read_line read_to_end read_to_string remove repeat
    replace rev send sleep sort split split_once spawn starts_with strip_prefix sum take to_owned to_string
    trim trim_end trim_start try_into unwrap unwrap_or unwrap_or_default unwrap_or_else write write_all
    append exit metadata rename scope with_capacity saturating_sub checked_sub then fetch_add load store
    """.split()
)


def dependency_names(project):
    data = tomllib.loads((project / "Cargo.toml").read_text())
    names = {name.replace("-", "_"): name for name in data.get("dependencies", {})}
    if "leyline_http" in names:
        names["leyline"] = names["leyline_http"]
    return names


def rust_analyze(toks, library, project, source=""):
    deps = dependency_names(project)
    deps.setdefault("std", "std")
    idents = list(toks)
    roots = set()
    imported = set()
    used = set()
    std_names = set()
    i = 0
    n = len(idents)
    while i < n:
        kind, text = idents[i]
        if kind == "ident" and text == "use" and (i == 0 or idents[i - 1][1] != "."):
            j = i + 1
            leaves = []
            while j < n and idents[j][1] != ";":
                if idents[j][0] == "ident":
                    following = idents[j + 1][1] if j + 1 < n else ""
                    if following != "::" and text != "self":
                        leaves.append(idents[j][1])
                    if j == i + 1:
                        roots.add(idents[j][1])
                j += 1
            imported.update(x for x in leaves if x not in ("as", "self", "crate"))
            if idents[i + 1][1] == "std":
                std_names.update(leaves)
            else:
                used.update(idents[k][1] for k in range(i + 1, j) if idents[k][0] == "ident")
            i = j
        elif kind == "ident" and i + 1 < n and idents[i + 1][1] == "::":
            prev = idents[i - 1][1] if i else ""
            if prev != "::" and text in deps:
                roots.add(text)
                imported.add(text)
        i += 1
    in_std_path = False
    for k in range(1, n):
        if idents[k][0] != "ident":
            continue
        if idents[k - 1][1] == "::":
            if in_std_path:
                std_names.add(idents[k][1])
                continue
        elif idents[k][1] == "std" and k + 1 < n and idents[k + 1][1] == "::":
            in_std_path = True
            continue
        else:
            in_std_path = False
        if idents[k - 1][1] in (".", "::"):
            used.add(idents[k][1])
    roots &= set(deps)
    packages = {deps[r] for r in roots}
    used |= imported
    return imported, packages, used, (RUST_STD | std_names) - VERBS


def python_std():
    names = set()
    for kind in (str, bytes, list, dict, set, tuple, int, float):
        names.update(dir(kind))
    import io

    names.update(dir(io.BufferedWriter))
    return names - VERBS


def python_analyze(toks, library, project, source=""):
    imported = set()
    roots = set()
    used = set()
    std_names = set()
    for node in ast.walk(ast.parse(source)):
        if isinstance(node, ast.Import):
            for alias in node.names:
                imported.add(alias.name)
                roots.add(alias.name.split(".")[0])
                if alias.name.split(".")[0] in sys.stdlib_module_names:
                    std_names.add(alias.name)
        elif isinstance(node, ast.ImportFrom):
            root = (node.module or "").split(".")[0]
            roots.add(root)
            imported.update(alias.name for alias in node.names)
            if root in sys.stdlib_module_names:
                std_names.update(alias.name for alias in node.names)
    for k in range(1, len(toks)):
        kind, text = toks[k]
        if kind != "ident":
            continue
        if toks[k - 1][1] == ".":
            if toks[k - 2][1] not in std_names:
                used.add(text)
        elif k + 1 < len(toks) and toks[k + 1][1] == "=" and toks[k - 1][1] in ("(", ","):
            used.add(text)
    used |= {name.split(".")[-1] for name in imported}
    return imported, roots, used, python_std() | std_names


NODE_STD = None


def node_std():
    global NODE_STD
    if NODE_STD is None:
        NODE_STD = set(json.loads((ROOT / "node" / "builtin-names.json").read_text())) - VERBS
    return NODE_STD


def node_analyze(toks, library, project, source=""):
    imported = set()
    roots = set()
    used = set()
    node_names = set()
    n = len(toks)
    i = 0
    while i < n:
        kind, text = toks[i]
        if kind == "ident" and text == "import" and (i + 1 < n and toks[i + 1][1] != "("):
            j = i + 1
            names = []
            while j < n and not (toks[j][0] == "ident" and toks[j][1] == "from") and toks[j][0] != "string":
                if toks[j][0] == "ident" and toks[j][1] != "as":
                    if toks[j - 1][1] != "as":
                        names.append(toks[j][1])
                j += 1
            if j < n and toks[j][1] == "from":
                j += 1
            spec = toks[j][1].strip("'\"`")
            roots.add(spec)
            imported.update(names or [spec])
            if spec.startswith("node:"):
                node_names.update(names)
            i = j
        i += 1
    for k in range(1, n):
        kind, text = toks[k]
        if kind != "ident":
            continue
        if toks[k - 1][1] in (".", "?."):
            used.add(text)
        elif k + 1 < n and toks[k + 1][1] == ":" and toks[k - 1][1] in ("{", ","):
            used.add(text)
        elif text in imported:
            used.add(text)
    used |= imported
    return imported, roots, used, node_std() | node_names


ANALYZE = {"rust": rust_analyze, "python": python_analyze, "node": node_analyze}


def external_packages(lang, roots, library, bundled):
    if lang == "rust":
        names = {r for r in roots if r != "std"}
    elif lang == "python":
        names = {r for r in roots if r not in sys.stdlib_module_names}
    else:
        names = set()
        for root in roots:
            if root.startswith("node:"):
                continue
            parts = root.split("/")
            names.add("/".join(parts[:2]) if root.startswith("@") else parts[0])
    return sorted(names), sorted(n for n in names if n != library and n not in bundled)


def score(path, lang, library, bundled, surface, project=None):
    source = Path(path).read_text()
    toks = tokens(source, lang)
    imported, roots, used, std = ANALYZE[lang](toks, library, project, source)
    packages, extras = external_packages(lang, roots, library, bundled)
    names = set()
    for package in packages:
        names |= surface.get(package, set())
    if library in surface:
        names |= surface[library]
    concepts = sorted(x for x in used if x in names and x not in std)
    return {
        "lines": code_lines(source, lang),
        "tokens": len(toks),
        "imports": len(imported),
        "extra_deps": len(extras),
        "extras": extras,
        "concepts": len(concepts),
        "concept_names": concepts,
    }


def extract_surface(lang, files):
    names = set()
    for file in files:
        try:
            text = Path(file).read_text(errors="replace")
        except OSError:
            continue
        if lang == "rust":
            names.update(re.findall(r"\bpub\s+(?:const\s+|unsafe\s+|async\s+)*(?:fn|struct|enum|trait|type|mod|const|static)\s+(\w+)", text))
            names.update(re.findall(r"\bpub use [^;]*?(\w+)\s*;", text))
            for block in re.findall(r"\bpub\s+use\s+[^;]*\{([^;]*)\}\s*;", text):
                names.update(re.findall(r"\b([A-Za-z_]\w*)\b", block))
            for body in re.findall(r"\bpub\s+enum\s+\w+[^{]*\{(.*?)\n\}", text, re.S):
                names.update(re.findall(r"^\s*([A-Z]\w*)", body, re.M))
            names.update(re.findall(r"\b([A-Z][A-Z_]{2,})\b\s*[,:=)]", text))
        elif lang == "python":
            names.update(re.findall(r"^\s*(?:async\s+)?def\s+([A-Za-z]\w*)", text, re.M))
            names.update(re.findall(r"^\s*class\s+([A-Za-z]\w*)", text, re.M))
            names.update(re.findall(r"\bself\.([A-Za-z]\w*)\s*(?::[^=\n]+)?=", text))
            names.update(re.findall(r"^\s*([A-Za-z]\w*)\s*:\s*[A-Za-z\"']", text, re.M))
            names.update(re.findall(r"^([A-Za-z]\w*)\s*=", text, re.M))
            for params in re.findall(r"def\s+\w+\s*\((.*?)\)\s*(?:->[^:]*)?:", text, re.S):
                names.update(re.findall(r"(?:^|,)\s*\*{0,2}([A-Za-z]\w*)", params))
        else:
            names.update(re.findall(r"\b(?:class|interface|type|function|const|namespace|enum)\s+([A-Za-z_$][\w$]*)", text))
            names.update(re.findall(r"^\s*(?:readonly\s+|static\s+|async\s+|get\s+|set\s+)*([A-Za-z_$][\w$]*)\??\s*[:(<]", text, re.M))
            names.update(re.findall(r"\bexport\s*\{([^}]*)\}", text) and re.findall(r"[A-Za-z_$][\w$]*", " ".join(re.findall(r"\bexport\s*\{([^}]*)\}", text))))
    return names
