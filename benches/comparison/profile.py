"""Summaries for profile.sh: cycles by layer, allocations and load sources per request."""

import collections
import re
import sys

LAYERS = [
    ("crypto", r"bssl::|LEYLINE_|aead_|aws_lc|_gcm|aes_"),
    ("tls-io", r"RecordStream|rustls|tokio_rustls|leyline_bssl_tokio"),
    ("http2", r"leyline::h2::|^h2::|<h2::"),
    ("http1", r"leyline::pool::h1|hyper::proto"),
    ("pool", r"leyline::pool::|hyper_util::client::legacy::pool"),
    ("session", r"leyline::core::|leyline::trace|reqwest::"),
    ("tokio-io", r"tokio::runtime::io|tokio::net|mio::"),
    ("tokio-time", r"tokio::runtime::time|tokio::time"),
    ("tokio-sync", r"tokio::sync"),
    ("tokio-sched", r"tokio::runtime::"),
    ("bench", r"cmp_client|bench_mode"),
]
HELPERS = r"malloc|_int_free|cfree|realloc|memmove|memcpy|memcmp|bytes::|http::|alloc::|core::"


def frames(stream):
    for block in stream.read().split("\n\n"):
        lines = [line.strip().split(None, 1) for line in block.strip().split("\n") if line.strip()]
        yield [parts[1] if len(parts) > 1 else "" for parts in lines]


def clean(name):
    for _ in range(8):
        name = re.sub(r"(?<=[\w\]:])<[^<>]*>", "", name)
    return name[:100]


def layer_of(name):
    path = clean(name)
    return next((layer for layer, pattern in LAYERS if re.search(pattern, path)), None)


def layers(cycles_per_request):
    owner = collections.Counter()
    leaf = collections.Counter()
    total = 0
    for stack in frames(sys.stdin):
        if not stack:
            continue
        total += 1
        found = next((layer for layer in map(layer_of, stack) if layer), "other")
        owner[found] += 1
        name = stack[0] if not re.search(HELPERS, stack[0]) else stack[0] + " in " + next(
            (frame for frame in stack[1:] if layer_of(frame)), "?"
        )
        leaf[clean(name)] += 1
    scale = cycles_per_request / max(total, 1)
    print("cycles/req by layer: " + "  ".join(f"{k}={v * scale:.0f}" for k, v in owner.most_common()))
    print("hot functions (self cycles/req):")
    for name, count in leaf.most_common(25):
        print(f"  {count * scale:7.0f}  {name}")


def read_sizes(path):
    sizes = collections.Counter()
    for line in open(path):
        if line.startswith("SIZE "):
            _, size, count = line.split()
            sizes[int(size)] = int(count)
    return sizes


def allocs(short_run, long_run, requests):
    low, high = read_sizes(short_run), read_sizes(long_run)
    per = {size: (high[size] - low[size]) / requests for size in high}
    per = {size: count for size, count in per.items() if count >= 0.05}
    print(
        f"allocations/req {sum(per.values()):.2f}  bytes/req {sum(s * c for s, c in per.items()):.0f}  "
        + "sizes: " + " ".join(f"{size}x{count:.2g}" for size, count in sorted(per.items()))
    )


def loads(requests):
    sources = collections.Counter()
    for line in sys.stdin:
        if "OP LOAD" not in line or re.search(r"\sffff[0-9a-f]+\s*$", line):
            continue
        level = re.search(r"LVL ([^|]*)", line)
        if level:
            sources[level.group(1).strip()] += 1
    scale = 50003 / requests
    print("user loads/req by source: " + "  ".join(f"{k}={v * scale:.0f}" for k, v in sources.most_common()))


if __name__ == "__main__":
    command, args = sys.argv[1], sys.argv[2:]
    if command == "layers":
        layers(float(args[0]))
    elif command == "allocs":
        allocs(args[0], args[1], int(args[2]))
    elif command == "loads":
        loads(int(args[0]))
