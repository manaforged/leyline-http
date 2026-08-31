#!/usr/bin/env python3
"""One-line rustdoc / SAFETY gate for first-party Leyline Rust."""
from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

ROOTS = [
    Path("crates/leyline/src"),
    Path("crates/leyline/examples"),
    Path("crates/leyline/tests"),
    Path("crates/leyline-ffi/src"),
    Path("crates/leyline-node/src"),
    Path("crates/leyline-python/src"),
    Path("fuzz/fuzz_targets"),
]
SKIP_PARTS = ("leyline-quiche", "leyline-bssl")
GATE = "runtime-bounds: allow"
HANG = re.compile(
    r"\b(a|an|the|and|or|of|to|for|with|plus|from|that|which|when|via)\s*$",
    re.I,
)
CUT_END = re.compile(
    r"(HTTP/1\.|TLS 1\.|OkHttp 4\.|Brave 1\.|Node\.)\s*$"
)


def _ident_continue(ch: str) -> bool:
    return ch.isalnum() or ch == "_"


def _ident_start(ch: str) -> bool:
    return ch.isalpha() or ch == "_"


def first_sentence(text: str) -> str:
    text = " ".join(text.split())
    if not text:
        return text
    i = 0
    n = len(text)
    paren = bracket = 0
    in_tick = in_dq = False
    while i < n:
        ch = text[i]
        rest = text[i:]
        if rest.lower().startswith("e.g.") or rest.lower().startswith("i.e."):
            i += 4
            continue
        if rest.lower().startswith("etc.") or rest.lower().startswith("vs."):
            i += 4
            continue
        if ch == "`":
            in_tick = not in_tick
            i += 1
            continue
        if ch == '"' and not in_tick:
            in_dq = not in_dq
            i += 1
            continue
        if not in_tick and not in_dq:
            if ch == "(":
                paren += 1
            elif ch == ")" and paren:
                paren -= 1
            elif ch == "[":
                bracket += 1
            elif ch == "]" and bracket:
                bracket -= 1
            elif ch == "." and i > 0 and text[i - 1].isdigit() and i + 1 < n and text[i + 1].isdigit():
                i += 1
                continue
            elif ch in ".!?" and paren == 0 and bracket == 0:
                nxt = text[i + 1 : i + 2]
                if nxt == "" or nxt.isspace():
                    return text[: i + 1].strip()
        i += 1
    return text.strip()


def truncated(body: str) -> bool:
    if not body:
        return True
    if HANG.search(body):
        return True
    if body.endswith(",") or body.endswith("\u2014") or body.endswith("-"):
        return True
    if CUT_END.search(body):
        return True
    ticks = body.count("`")
    if ticks % 2:
        return True
    if body.count("(") != body.count(")"):
        return True
    if body.count("[") != body.count("]"):
        return True
    if body.count('"') % 2:
        return True
    return False


def kind_line(s: str) -> str | None:
    t = s.lstrip()
    if t.startswith("///") or t.startswith("//!"):
        return "doc"
    if t.startswith("//"):
        body = t[2:].lstrip()
        if body.startswith("SAFETY:"):
            return "safety"
        if GATE in body:
            return "gate"
        return "line"
    return None


def iter_code_lines(src: str):
    """Yield (is_code, line_without_nl, newline) with strings kept as code."""
    n = len(src)
    i = 0
    buf: list[str] = []
    prev_ident = False
    at_bol = True
    pending_indent: list[str] = []

    def peek(k: int = 0) -> str:
        j = i + k
        return src[j] if j < n else ""

    def flush_code():
        nonlocal buf
        if buf:
            chunk = "".join(buf)
            buf = []
            parts = chunk.splitlines(keepends=True)
            for p in parts:
                if p.endswith("\r\n"):
                    yield True, p[:-2], "\r\n"
                elif p.endswith("\n"):
                    yield True, p[:-1], "\n"
                elif p.endswith("\r"):
                    yield True, p[:-1], "\r"
                else:
                    yield True, p, ""

    def read_string(quote: str) -> str:
        nonlocal i, prev_ident
        out = [quote]
        i += 1
        while i < n:
            ch = src[i]
            out.append(ch)
            i += 1
            if ch == "\\":
                if i < n:
                    out.append(src[i])
                    i += 1
                continue
            if ch == quote:
                break
        prev_ident = False
        return "".join(out)

    def read_raw() -> str | None:
        nonlocal i, prev_ident
        save = i
        out: list[str] = []
        if peek() in "bc":
            out.append(src[i])
            i += 1
        if peek() != "r":
            i = save
            return None
        out.append("r")
        i += 1
        hashes = 0
        while peek() == "#":
            out.append("#")
            i += 1
            hashes += 1
        if peek() != '"':
            i = save
            return None
        out.append('"')
        i += 1
        close = '"' + ("#" * hashes)
        while i < n:
            if src.startswith(close, i):
                out.append(close)
                i += len(close)
                prev_ident = False
                return "".join(out)
            out.append(src[i])
            i += 1
        prev_ident = False
        return "".join(out)

    def read_char_or_lifetime() -> str:
        nonlocal i, prev_ident
        save = i
        i += 1
        if i >= n:
            prev_ident = False
            return "'"
        nxt = src[i]
        if nxt == "\\":
            buf2 = ["'", nxt]
            i += 1
            if i < n:
                buf2.append(src[i])
                i += 1
            if i < n and src[i] == "'":
                buf2.append("'")
                i += 1
            prev_ident = False
            return "".join(buf2)
        if i + 1 < n and src[i + 1] == "'":
            text = src[save : i + 2]
            i = i + 2
            prev_ident = False
            return text
        if _ident_start(nxt):
            j = i
            while j < n and _ident_continue(src[j]):
                j += 1
            text = src[save:j]
            i = j
            prev_ident = True
            return text
        prev_ident = False
        return "'"

    while i < n:
        ch = src[i]
        if at_bol and ch in " \t":
            pending_indent.append(ch)
            i += 1
            continue
        if ch == "/" and peek(1) == "/":
            yield from flush_code()
            start = i
            while i < n and src[i] not in "\n\r":
                i += 1
            comment = "".join(pending_indent) + src[start:i]
            pending_indent = []
            nl = ""
            if i < n and src[i] == "\r":
                nl += "\r"
                i += 1
            if i < n and src[i] == "\n":
                nl += "\n"
                i += 1
            yield False, comment, nl
            prev_ident = False
            at_bol = True
            continue
        if ch == "/" and peek(1) == "*":
            start = i
            i += 2
            while i < n - 1:
                if src[i] == "*" and src[i + 1] == "/":
                    i += 2
                    break
                i += 1
            block = src[start:i]
            if block.startswith("/**") or block.startswith("/*!"):
                yield False, block, ""
            prev_ident = False
            continue
        if pending_indent:
            buf.extend(pending_indent)
            pending_indent = []
            at_bol = False
        if not prev_ident and ch in "bcr":
            raw = read_raw()
            if raw is not None:
                buf.append(raw)
                continue
        if ch == '"':
            buf.append(read_string('"'))
            continue
        if ch == "'":
            buf.append(read_char_or_lifetime())
            continue
        if pending_indent:
            buf.extend(pending_indent)
            pending_indent = []
        buf.append(ch)
        i += 1
        at_bol = ch in "\n\r"
        prev_ident = _ident_continue(ch)
    if pending_indent:
        buf.extend(pending_indent)
    yield from flush_code()


def scan_src(src: str, path: Path) -> list[str]:
    hits: list[str] = []
    line_no = 1
    pending_doc: list[tuple[int, str]] = []
    pending_safety: list[tuple[int, str]] = []

    def flush_doc():
        nonlocal pending_doc
        if not pending_doc:
            return
        start = pending_doc[0][0]
        if len(pending_doc) > 1:
            hits.append(f"{path}:{start}: doc comment is {len(pending_doc)} lines")
        body = pending_doc[0][1].lstrip()
        body = body[3:].strip() if body.startswith("///") or body.startswith("//!") else body
        if truncated(body):
            hits.append(f"{path}:{start}: rustdoc line is truncated")
        pending_doc = []

    def flush_safety():
        nonlocal pending_safety
        if not pending_safety:
            return
        start = pending_safety[0][0]
        if len(pending_safety) > 1:
            hits.append(f"{path}:{start}: safety comment is {len(pending_safety)} lines")
        body = pending_safety[0][1].lstrip()[2:].strip()
        if truncated(body):
            hits.append(f"{path}:{start}: SAFETY line is truncated")
        pending_safety = []

    for is_code, text, nl in iter_code_lines(src):
        if is_code:
            flush_doc()
            flush_safety()
            line_no += text.count("\n") + (1 if nl else (0 if "\n" not in text else 0))
            if nl:
                line_no += nl.count("\n")
            elif "\n" in text:
                pass
            else:
                if nl == "" and "\n" not in text:
                    if text:
                        pass
            continue
        k = kind_line(text)
        if k == "doc":
            flush_safety()
            pending_doc.append((line_no, text))
        elif k == "safety":
            flush_doc()
            pending_safety.append((line_no, text))
        elif k == "line":
            if pending_safety:
                pending_safety.append((line_no, text))
            else:
                flush_doc()
                hits.append(f"{path}:{line_no}: // is not rustdoc or SAFETY")
        elif k == "gate":
            flush_doc()
            flush_safety()
        else:
            flush_doc()
            flush_safety()
        line_no += 1 if (nl or True) else 0
        if nl:
            line_no += max(nl.count("\n") - 1, 0)
    flush_doc()
    flush_safety()
    return hits


def scan_file(path: Path) -> list[str]:
    return scan_src(path.read_text(encoding="utf-8"), path)


def collapse_text(src: str) -> str:
    out: list[str] = []
    doc_run: list[tuple[str, str]] = []
    safety_run: list[tuple[str, str]] = []

    def emit_doc():
        nonlocal doc_run
        if not doc_run:
            return
        indent_nl = doc_run[0]
        indent = re.match(r"^[ \t]*", indent_nl[0]).group(0)
        marker = "//! " if indent_nl[0].lstrip().startswith("//!") else "/// "
        bodies = []
        for text, _nl in doc_run:
            s = text.lstrip()
            if s.startswith("///"):
                rest = s[3:].strip()
            elif s.startswith("//!"):
                rest = s[3:].strip()
            else:
                rest = ""
            if rest and not rest.startswith("#"):
                bodies.append(rest)
        sent = first_sentence(" ".join(bodies)) if bodies else ""
        if not sent and bodies:
            sent = bodies[0]
        out.append(f"{indent}{marker}{sent}{doc_run[-1][1] or chr(10)}")
        doc_run = []

    def emit_safety():
        nonlocal safety_run
        if not safety_run:
            return
        indent = re.match(r"^[ \t]*", safety_run[0][0]).group(0)
        parts = []
        for text, _nl in safety_run:
            s = text.lstrip()[2:].strip()
            if s.startswith("SAFETY:"):
                parts.append(s)
            else:
                parts.append(s)
        joined = " ".join(parts)
        if not joined.upper().startswith("SAFETY:"):
            joined = "SAFETY: " + joined
        out.append(f"{indent}// {joined}{safety_run[-1][1] or chr(10)}")
        safety_run = []

    for is_code, text, nl in iter_code_lines(src):
        if is_code:
            emit_doc()
            emit_safety()
            out.append(text + nl)
            continue
        k = kind_line(text)
        if k == "doc":
            emit_safety()
            doc_run.append((text, nl))
        elif k == "safety":
            emit_doc()
            safety_run.append((text, nl))
        elif k == "line" and safety_run:
            safety_run.append((text, nl))
        elif k == "gate":
            emit_doc()
            emit_safety()
            out.append(text + nl)
        elif k == "line":
            emit_doc()
            emit_safety()
        else:
            emit_doc()
            emit_safety()
            if text.startswith("/**") or text.startswith("/*!"):
                bodies = re.sub(r"^/\*\*?\!?|\*/$", "", text, flags=re.S)
                sent = first_sentence(" ".join(bodies.split()))
                out.append(f"/// {sent}\n")
    emit_doc()
    emit_safety()
    return "".join(out)


def iter_rs(root: Path) -> list[Path]:
    if not root.exists():
        return []
    files = []
    if root.is_file():
        return [root]
    for path in root.rglob("*.rs"):
        if any(part in path.parts for part in SKIP_PARTS):
            continue
        files.append(path)
    return files


def check(roots: list[Path]) -> list[str]:
    hits: list[str] = []
    for root in roots:
        for path in iter_rs(root):
            hits.extend(scan_file(path))
    return hits


def fix(roots: list[Path]) -> int:
    n = 0
    for root in roots:
        for path in iter_rs(root):
            src = path.read_text(encoding="utf-8")
            got = collapse_text(src)
            if got != src:
                path.write_text(got, encoding="utf-8")
                n += 1
    return n


def from_git(rev: str, roots: list[Path]) -> int:
    n = 0
    for root in roots:
        for path in iter_rs(root):
            rel = path.as_posix()
            proc = subprocess.run(
                ["git", "show", f"{rev}:{rel}"],
                capture_output=True,
                text=True,
            )
            if proc.returncode != 0:
                continue
            got = collapse_text(proc.stdout)
            if path.read_text(encoding="utf-8") != got:
                path.write_text(got, encoding="utf-8")
                n += 1
    return n


def self_test() -> None:
    cases_scan = [
        ("/// Does the thing.\nfn x() {}\n", []),
        ("/// Does the thing.\n/// Also this.\nfn x() {}\n", ["doc comment is 2 lines"]),
        ("//! Node.js N-API bindings for Leyline.\n", []),
        ("//! Node.\n", ["truncated"]),
        ("/// HTTP/1.1 request.\n", []),
        ("/// HTTP/1.\n", ["truncated"]),
        ("/// Allow TLS 1.2 on TCP.\n", []),
        ("/// Allow TLS 1.\n", ["truncated"]),
        ("/// Range `[0.0, 1.0]`.\n", []),
        ("/// Range `[0.\n", ["truncated"]),
        ("/// (RFC 9113 Section 6.2).\n", []),
        ("// SAFETY: fd is live.\nunsafe { }\n", []),
        ("// SAFETY: fd is live\n// and val outlives the call.\nunsafe { }\n", ["safety comment is 2 lines"]),
        ("// no\nfn x() {}\n", ["// is not rustdoc or SAFETY"]),
        ("// runtime-bounds: allow\nuse x;\n", []),
        ('let s = "/// not a comment";\n', []),
        ('let s = r#"\n// not a comment\n"#;\n', []),
    ]
    failed = 0
    tmp = Path("/tmp/check-comments-self.rs")
    for src, expect_sub in cases_scan:
        tmp.write_text(src, encoding="utf-8")
        hits = scan_file(tmp)
        joined = " ".join(hits)
        ok = True
        if not expect_sub and hits:
            ok = False
        for sub in expect_sub:
            if sub not in joined:
                ok = False
        if not ok:
            failed += 1
            print("SCAN FAIL", repr(src), hits)
    collapse_cases = [
        (
            "/// HTTP/1.1 is the protocol.\n/// Extra paragraph.\nfn x() {}\n",
            "/// HTTP/1.1 is the protocol.\nfn x() {}\n",
        ),
        (
            "/// Range `[0.0, 1.0]` for jitter.\n",
            "/// Range `[0.0, 1.0]` for jitter.\n",
        ),
        (
            "// SAFETY: fd is live\n// and val outlives the call.\nunsafe {}\n",
            "// SAFETY: fd is live and val outlives the call.\nunsafe {}\n",
        ),
        (
            'let s = r#"\n// keep me\n"#;\n',
            'let s = r#"\n// keep me\n"#;\n',
        ),
        (
            "//! Node.js N-API bindings for Leyline.\n//! Thin adapter.\n",
            "//! Node.js N-API bindings for Leyline.\n",
        ),
    ]
    for src, want in collapse_cases:
        got = collapse_text(src)
        if got != want:
            failed += 1
            print("COLLAPSE FAIL")
            print(" SRC", repr(src))
            print(" GOT", repr(got))
            print(" WNT", repr(want))
    if failed:
        raise SystemExit(f"{failed} self-tests failed")
    print("self-test ok")


def main(argv: list[str]) -> int:
    if "--self-test" in argv:
        self_test()
        return 0
    roots = ROOTS
    if "--from-git" in argv:
        idx = argv.index("--from-git")
        if idx + 1 >= len(argv):
            print("usage: check-comments.py --from-git REV", file=sys.stderr)
            return 2
        n = from_git(argv[idx + 1], roots)
        print(f"rebuilt {n} files from {argv[idx + 1]}")
    elif "--fix" in argv:
        n = fix(roots)
        print(f"collapsed {n} files")
    hits = check(roots)
    if hits:
        print("\n".join(hits[:80]))
        if len(hits) > 80:
            print(f"... {len(hits) - 80} more")
        print(f"{len(hits)} comment lint failures", file=sys.stderr)
        return 1
    print("comment lint ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
