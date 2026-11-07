import email.parser
import email.policy
import hashlib
import json
import random
import re
import socket
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = 18743
BASE = f"http://127.0.0.1:{PORT}"
LARGE_SIZE = 64 * 1024 * 1024
LARGE_ETAG = '"large-v1"'
LARGE_DROPS = (0.40, 0.70)
UPLOAD_SIZE = 256 * 1024
FAN_DELAY = 0.03
SLOW_DELAY = 3.0
PROXY_HOST = "ergonomics.invalid"
LOOP_CAP = 5


def pattern(size, seed):
    return random.Random(seed).randbytes(size)


LARGE = pattern(LARGE_SIZE, 1)
UPLOAD = pattern(UPLOAD_SIZE, 2)
LARGE_SHA = hashlib.sha256(LARGE).hexdigest()
UPLOAD_SHA = hashlib.sha256(UPLOAD).hexdigest()


class State:
    def __init__(self):
        self.lock = threading.Lock()
        self.log = []
        self.in_flight = 0
        self.peak = 0
        self.fan_seen = set()
        self.flaky = []
        self.large_sent = 0
        self.large_gets = []
        self.sse = []
        self.loop_hits = 0
        self.slow_at = None
        self.facts = {}


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    tunnel_host = None

    def log_message(self, *args):
        pass

    @property
    def state(self):
        return self.server.state

    def body(self):
        if self.headers.get("Transfer-Encoding", "").lower() == "chunked":
            out = b""
            while True:
                size = int(self.rfile.readline().strip().split(b";")[0], 16)
                if size == 0:
                    self.rfile.readline()
                    return out
                out += self.rfile.read(size)
                self.rfile.readline()
        return self.rfile.read(int(self.headers.get("Content-Length", 0)))

    def reply(self, status, payload=b"", headers=()):
        if isinstance(payload, (dict, list)):
            payload = json.dumps(payload).encode()
            headers = [("Content-Type", "application/json"), *headers]
        self.send_response(status)
        for name, value in headers:
            self.send_header(name, value)
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(payload)

    def do_CONNECT(self):
        self.tunnel_host = self.path.split(":")[0]
        self.send_response(200, "Connection Established")
        self.end_headers()
        self.close_connection = False

    def route(self):
        if self.path.startswith("http://"):
            match = re.match(r"http://([^/]+)(/.*)", self.path)
            return match.group(2), match.group(1), "absolute"
        if self.tunnel_host:
            return self.path, self.tunnel_host, "connect"
        return self.path, self.headers.get("Host", ""), "origin"

    def dispatch(self):
        path, host, form = self.route()
        path = path.split("?")[0]
        for prefix, handler in ROUTES:
            if path.startswith(prefix):
                return handler(self, path, host, form)
        return self.reply(404, {"error": "no route"})

    do_GET = do_POST = do_HEAD = dispatch


def t01(h, path, host, form):
    with h.state.lock:
        h.state.facts["t01_gets"] = h.state.facts.get("t01_gets", 0) + 1
    h.reply(200, {"id": 42, "name": "ada"})


def t02_create(h, path, host, form):
    raw = h.body()
    try:
        body = json.loads(raw)
    except ValueError:
        body = None
    h.state.facts["t02_body_ok"] = body == {"name": "ada", "age": 36}
    h.state.facts["t02_type_ok"] = h.headers.get("Content-Type", "").startswith("application/json")
    h.reply(201, {"id": 7})


def t02_slow(h, path, host, form):
    h.body()
    h.state.slow_at = time.monotonic()
    time.sleep(SLOW_DELAY)
    try:
        h.reply(201, {"id": 8})
    except OSError:
        h.close_connection = True


def t03_login(h, path, host, form):
    try:
        body = json.loads(h.body())
    except ValueError:
        body = None
    h.state.facts["t03_login_ok"] = body == {"user": "ada", "password": "secret"}
    h.reply(200, {"ok": True}, [("Set-Cookie", "sid=s3cr3t; Path=/; HttpOnly")])


def t03_me(h, path, host, form):
    cookie = h.headers.get("Cookie", "")
    sent = "sid=s3cr3t" in cookie
    h.state.facts["t03_cookie_sent"] = sent
    if sent:
        h.reply(200, {"user": "ada"})
    else:
        h.reply(401, {"error": "no session"})


def t04(h, path, host, form):
    with h.state.lock:
        h.state.flaky.append(time.monotonic())
        attempt = len(h.state.flaky)
    if attempt == 1:
        h.reply(503, b"busy", [("Retry-After", "1")])
    elif attempt == 2:
        h.reply(503, b"busy")
    else:
        h.reply(200, {"attempts": attempt})


def t05(h, path, host, form):
    state = h.state
    index = int(path.rsplit("/", 1)[1])
    with state.lock:
        state.in_flight += 1
        state.peak = max(state.peak, state.in_flight)
        state.fan_seen.add(index)
    time.sleep(FAN_DELAY)
    with state.lock:
        state.in_flight -= 1
    h.reply(404 if index % 10 == 3 else 200, b"x")


def t06(h, path, host, form):
    state = h.state
    range_header = h.headers.get("Range")
    start = 0
    if range_header:
        match = re.fullmatch(r"bytes=(\d+)-", range_header)
        if not match:
            return h.reply(400, b"bad range")
        start = int(match.group(1))
    if_range = h.headers.get("If-Range")
    if if_range and if_range != LARGE_ETAG:
        start = 0
    common = [("ETag", LARGE_ETAG), ("Accept-Ranges", "bytes")]
    if h.command == "HEAD":
        return head_large(h, common)
    if start >= LARGE_SIZE:
        return h.reply(416, b"", [("Content-Range", f"bytes */{LARGE_SIZE}")])
    with state.lock:
        turn = len(state.large_gets)
        state.large_gets.append(start)
    stop = int(LARGE_SIZE * LARGE_DROPS[turn]) if turn < len(LARGE_DROPS) else LARGE_SIZE
    stop = max(stop, start)
    if start > 0:
        h.send_response(206)
        h.send_header("Content-Range", f"bytes {start}-{LARGE_SIZE - 1}/{LARGE_SIZE}")
    else:
        h.send_response(200)
    for name, value in common:
        h.send_header(name, value)
    h.send_header("Content-Length", str(LARGE_SIZE - start))
    h.end_headers()
    view = memoryview(LARGE)[start:stop]
    try:
        for offset in range(0, len(view), 1 << 20):
            chunk = view[offset : offset + (1 << 20)]
            h.wfile.write(chunk)
            with state.lock:
                state.large_sent += len(chunk)
    except OSError:
        pass
    if stop < LARGE_SIZE:
        h.close_connection = True
        h.wfile.flush()
        h.connection.shutdown(socket.SHUT_RDWR)


def head_large(h, common):
    h.send_response(200)
    for name, value in common:
        h.send_header(name, value)
    h.send_header("Content-Length", str(LARGE_SIZE))
    h.end_headers()


def t07(h, path, host, form):
    raw = h.body()
    content_type = h.headers.get("Content-Type", "")
    message = email.parser.BytesParser(policy=email.policy.HTTP).parsebytes(
        b"Content-Type: " + content_type.encode() + b"\r\n\r\n" + raw
    )
    facts = h.state.facts
    facts["t07_multipart"] = content_type.startswith("multipart/form-data")
    note = None
    size = None
    if message.is_multipart():
        for part in message.iter_parts():
            name = part.get_param("name", header="content-disposition")
            data = part.get_payload(decode=True)
            if name == "note":
                note = data.decode()
            elif name == "file":
                size = len(data)
                facts["t07_file_sha_ok"] = hashlib.sha256(data).hexdigest() == UPLOAD_SHA
                facts["t07_filename"] = part.get_filename()
    facts["t07_note_ok"] = note == "hello"
    h.reply(200, {"size": size, "note": note})


def t08(h, path, host, form):
    with h.state.lock:
        turn = len(h.state.sse)
        h.state.sse.append(h.headers.get("Last-Event-ID"))
    h.close_connection = True
    h.send_response(200)
    h.send_header("Content-Type", "text/event-stream")
    h.send_header("Cache-Control", "no-cache")
    h.send_header("Connection", "close")
    h.end_headers()
    if turn == 0:
        chunks = [b"retry: 100\n\n", b"id: 1\ndata: a\n\n", b"id: 2\ndata: b\n\n"]
    elif h.headers.get("Last-Event-ID") == "2":
        chunks = [b"id: 3\ndata: c\n\n", b"event: done\ndata: end\n\n"]
    else:
        chunks = [b"id: 1\ndata: replay\n\n", b"event: done\ndata: end\n\n"]
    try:
        for chunk in chunks:
            h.wfile.write(chunk)
            h.wfile.flush()
            time.sleep(0.02)
    except OSError:
        pass


def t09(h, path, host, form):
    h.state.facts["t09_form"] = form
    h.state.facts["t09_host"] = host.split(":")[0]
    h.reply(200, {"via": "proxy"})


def t10_step(next_path, status):
    def handler(h, path, host, form):
        h.reply(status, b"", [("Location", next_path)])

    return handler


def t10_final(h, path, host, form):
    h.state.facts["t10_final_hit"] = True
    h.reply(200, {"ok": True})


def t10_loop(h, path, host, form):
    index = int(path.rsplit("/", 1)[1])
    with h.state.lock:
        h.state.loop_hits += 1
    h.reply(302, b"", [("Location", f"/t10/loop/{index + 1}")])


ROUTES = [
    ("/t01/user", t01),
    ("/t02/users", t02_create),
    ("/t02/slow", t02_slow),
    ("/t03/login", t03_login),
    ("/t03/me", t03_me),
    ("/t04/flaky", t04),
    ("/t05/item/", t05),
    ("/t06/large", t06),
    ("/t07/upload", t07),
    ("/t08/events", t08),
    ("/t09/hello", t09),
    ("/t10/start", t10_step("/t10/a", 302)),
    ("/t10/a", t10_step("/t10/b", 301)),
    ("/t10/b", t10_step("/t10/final", 302)),
    ("/t10/final", t10_final),
    ("/t10/loop/", t10_loop),
]


def check(task, state, ended_at=None):
    facts = state.facts
    gaps = [b - a for a, b in zip(state.flaky, state.flaky[1:])]
    checks = {
        "t01": lambda: {"get_seen": facts.get("t01_gets", 0) >= 1},
        "t02": lambda: {
            "body": facts.get("t02_body_ok") is True,
            "content_type": facts.get("t02_type_ok") is True,
            "slow_seen": state.slow_at is not None,
            "gave_up_early": state.slow_at is not None
            and ended_at is not None
            and ended_at - state.slow_at < SLOW_DELAY - 0.5,
        },
        "t03": lambda: {
            "login_body": facts.get("t03_login_ok") is True,
            "cookie_sent": facts.get("t03_cookie_sent") is True,
        },
        "t04": lambda: {
            "three_attempts": len(state.flaky) == 3,
            "retry_after_honored": len(gaps) >= 1 and 0.95 <= gaps[0] < 5,
            "backoff": len(gaps) >= 2 and 0.02 <= gaps[1] < 5,
        },
        "t05": lambda: {
            "all_requested": state.fan_seen == set(range(100)),
            "peak_at_most_10": state.peak <= 10,
            "peak_at_least_8": state.peak >= 8,
        },
        "t06": lambda: {
            "two_drops_survived": len(state.large_gets) >= 3,
            "resumed_with_range": all(start > 0 for start in state.large_gets[1:])
            and len(state.large_gets) >= 2,
            "no_full_redownload": state.large_sent <= LARGE_SIZE * 1.05,
        },
        "t07": lambda: {
            "multipart": facts.get("t07_multipart") is True,
            "file_hash": facts.get("t07_file_sha_ok") is True,
            "filename": facts.get("t07_filename") == "upload.bin",
            "field": facts.get("t07_note_ok") is True,
        },
        "t08": lambda: {
            "reconnected": len(state.sse) == 2,
            "last_event_id": len(state.sse) == 2 and state.sse[1] == "2",
        },
        "t09": lambda: {
            "via_proxy": facts.get("t09_form") in ("absolute", "connect"),
            "target_host": facts.get("t09_host") == PROXY_HOST,
        },
        "t10": lambda: {
            "final_reached": facts.get("t10_final_hit") is True,
            "cap_enforced": LOOP_CAP <= state.loop_hits <= LOOP_CAP + 2,
        },
    }
    return checks[task]()


def facts(task, state):
    gaps = [round(b - a, 3) for a, b in zip(state.flaky, state.flaky[1:])]
    return {
        "t04": {"gaps": gaps},
        "t05": {"peak": state.peak, "seen": len(state.fan_seen)},
        "t06": {"starts": state.large_gets, "sent": state.large_sent},
        "t08": {"last_event_ids": state.sse},
        "t09": {"form": state.facts.get("t09_form"), "host": state.facts.get("t09_host")},
        "t10": {"loop_hits": state.loop_hits},
    }.get(task, {})


class Fixture(ThreadingHTTPServer):
    daemon_threads = True
    request_queue_size = 256

    def __init__(self):
        self.allow_reuse_address = True
        super().__init__(("127.0.0.1", PORT), Handler)
        self.state = State()

    def reset(self):
        self.state = State()

    def handle_error(self, request, client_address):
        pass

    def start(self):
        threading.Thread(target=self.serve_forever, daemon=True).start()
