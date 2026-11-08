#!/usr/bin/env python3
import argparse
import asyncio
import collections
import concurrent.futures
import json
import pathlib
import subprocess
import threading

HERE = pathlib.Path(__file__).parent
BODY = b"BODY-0123456789"
REJECT = "reject"
ANY = "any"


def ok(status, body=b""):
    return (status, body)


def head(*fields, status=b"HTTP/1.1 200 OK"):
    return b"\r\n".join([status, *fields]) + b"\r\n\r\n"


HELLO = b"5\r\nhello\r\n0\r\n\r\n"
TABLE = [
    ("length", head(b"Content-Length: 5") + b"hello", [ok(200, b"hello")]),
    ("length ignores later bytes", head(b"Content-Length: 5") + b"helloEXTRA", [ok(200, b"hello")]),
    ("length short", head(b"Content-Length: 9") + b"hello", [REJECT]),
    ("length zero", head(b"Content-Length: 0") + b"hello", [ok(200)]),
    ("length padded", head(b"Content-Length:   5  ") + b"hello", [ok(200, b"hello")]),
    ("length repeated same", head(b"Content-Length: 5", b"Content-Length: 5") + b"hello", [ok(200, b"hello"), REJECT]),
    ("length list same", head(b"Content-Length: 5, 5") + b"hello", [ok(200, b"hello"), REJECT]),
    ("length repeated different", head(b"Content-Length: 5", b"Content-Length: 3") + b"hello", [REJECT]),
    ("length list different", head(b"Content-Length: 5, 3") + b"hello", [REJECT]),
    ("length plus sign", head(b"Content-Length: +5") + b"hello", [REJECT]),
    ("length negative", head(b"Content-Length: -5") + b"hello", [REJECT]),
    ("length hex", head(b"Content-Length: 0x5") + b"hello", [REJECT]),
    ("length empty", head(b"Content-Length: ") + b"hello", [REJECT]),
    ("length letters", head(b"Content-Length: 5a") + b"hello", [REJECT]),
    ("length overflow", head(b"Content-Length: 99999999999999999999999999") + b"hello", [REJECT]),
    ("no length closes", head() + b"hello", [ok(200, b"hello")]),
    ("http 1.0 closes", head(status=b"HTTP/1.0 200 OK") + b"hello", [ok(200, b"hello")]),
    ("no reason phrase", head(status=b"HTTP/1.1 200 ") + b"hello", [ok(200, b"hello")]),
    ("chunked", head(b"Transfer-Encoding: chunked") + HELLO, [ok(200, b"hello")]),
    ("chunked any case", head(b"Transfer-Encoding: ChUnKeD") + HELLO, [ok(200, b"hello")]),
    ("chunked hex size", head(b"Transfer-Encoding: chunked") + b"A\r\n0123456789\r\n0\r\n\r\n", [ok(200, b"0123456789")]),
    ("chunked extension", head(b"Transfer-Encoding: chunked") + b"5;a=b\r\nhello\r\n0\r\n\r\n", [ok(200, b"hello")]),
    ("chunked trailer", head(b"Transfer-Encoding: chunked") + b"5\r\nhello\r\n0\r\nx: y\r\n\r\n", [ok(200, b"hello")]),
    ("chunked ignores later bytes", head(b"Transfer-Encoding: chunked") + HELLO + b"EXTRA", [ok(200, b"hello")]),
    ("chunked truncated", head(b"Transfer-Encoding: chunked") + b"5\r\nhello\r\n", [REJECT]),
    ("chunked truncated in data", head(b"Transfer-Encoding: chunked") + b"9\r\nhello", [REJECT]),
    ("chunked bad size", head(b"Transfer-Encoding: chunked") + b"zz\r\nhello\r\n0\r\n\r\n", [REJECT]),
    ("chunked negative size", head(b"Transfer-Encoding: chunked") + b"-5\r\nhello\r\n0\r\n\r\n", [REJECT]),
    ("chunked overflow size", head(b"Transfer-Encoding: chunked") + b"ffffffffffffffffff\r\nhello\r\n0\r\n\r\n", [REJECT]),
    ("chunked missing data end", head(b"Transfer-Encoding: chunked") + b"5\r\nhelloXX0\r\n\r\n", [REJECT]),
    ("chunk beats length", head(b"Transfer-Encoding: chunked", b"Content-Length: 3") + HELLO, [ok(200, b"hello"), REJECT]),
    ("length before chunk", head(b"Content-Length: 3", b"Transfer-Encoding: chunked") + HELLO, [ok(200, b"hello"), REJECT]),
    ("not final chunked closes", head(b"Transfer-Encoding: chunked, gzip") + b"hello", [ok(200, b"hello"), REJECT]),
    ("unknown coding closes", head(b"Transfer-Encoding: gzip") + b"hello", [ok(200, b"hello"), REJECT]),
    ("near chunked closes", head(b"Transfer-Encoding: xchunked") + b"hello", [ok(200, b"hello"), REJECT]),
    ("204 has no body", head(b"Content-Length: 5", status=b"HTTP/1.1 204 No Content") + b"hello", [ok(204)]),
    ("304 has no body", head(b"Content-Length: 5", status=b"HTTP/1.1 304 Not Modified") + b"hello", [ok(304)]),
    ("204 ignores chunked", head(b"Transfer-Encoding: chunked", status=b"HTTP/1.1 204 No Content") + HELLO, [ok(204), REJECT]),
    ("100 then final", head(status=b"HTTP/1.1 100 Continue") + head(b"Content-Length: 5") + b"hello", [ok(200, b"hello")]),
    ("103 then final", head(status=b"HTTP/1.1 103 Early Hints") + head(b"Content-Length: 5") + b"hello", [ok(200, b"hello")]),
    ("bare newlines", b"HTTP/1.1 200 OK\nContent-Length: 5\n\nhello", [ok(200, b"hello"), REJECT]),
    ("folded field", head(b"X-A: 1", b" 2", b"Content-Length: 5") + b"hello", [ok(200, b"hello"), REJECT]),
    ("folded length", head(b"Content-Length:", b" 5") + b"hello", [ok(200, b"hello"), REJECT]),
    ("space before colon", head(b"Content-Length : 5") + b"hello", [ok(200, b"hello"), REJECT]),
    ("space before colon sets length", head(b"Content-Length : 3") + b"hello", [ok(200, b"hel"), REJECT]),
    ("tab before colon sets length", head(b"Content-Length\t: 3") + b"hello", [ok(200, b"hel"), REJECT]),
    ("space before colon sets chunked", head(b"Transfer-Encoding : chunked") + HELLO, [ok(200, b"hello"), REJECT]),
    ("folded length sets length", head(b"Content-Length:", b" 3") + b"hello", [ok(200, b"hel"), REJECT]),
    ("folded chunked sets chunked", head(b"Transfer-Encoding:", b" chunked") + HELLO, [ok(200, b"hello"), REJECT]),
    ("line after status is consumed", head(b" X-A: 1", b"Content-Length: 3") + b"hello", [ok(200, b"hel"), REJECT]),
    ("field line with no colon", head(b"no colon here", b"Content-Length: 3") + b"hello", [ok(200, b"hel"), REJECT]),
    ("nul in length name", head(b"Content-Length\x00: 3") + b"hello", [ok(200, b"hello"), REJECT]),
    ("bare carriage return in length", head(b"Content-Length: 3\r") + b"hello", [ok(200, b"hel"), REJECT]),
    ("carriage return splits head", b"HTTP/1.1 200 OK\r\nX-A: 1\r\n\rContent-Length: 3\r\n\r\nhello", [ok(200, b"hello"), ok(200, b"hel"), REJECT]),
    ("nul in field", head(b"X-A: a\x00b", b"Content-Length: 5") + b"hello", [ok(200, b"hello"), REJECT]),
    ("carriage return in field", head(b"X-A: a\rb", b"Content-Length: 5") + b"hello", [ok(200, b"hello"), REJECT]),
    ("empty reply", b"", [REJECT]),
    ("not http", b"hello there\r\n\r\n", [REJECT]),
    ("cut in head", b"HTTP/1.1 200 OK\r\nContent-Le", [REJECT]),
    ("status too short", head(status=b"HTTP/1.1 20 OK") + b"hello", [REJECT]),
    ("status too long", head(status=b"HTTP/1.1 2000 OK") + b"hello", [REJECT]),
]


def corpus_cases(folder):
    if not folder:
        return []
    root = pathlib.Path(folder)
    cases = []
    for path in sorted((root / "h1_head").iterdir()):
        raw = path.read_bytes()
        cases.append((f"head {path.name}", raw + BODY))
        cases.append((f"head+end {path.name}", raw + b"\r\n\r\n" + BODY))
    for path in sorted((root / "h1_chunked").iterdir()):
        cases.append((f"chunked {path.name}", head(b"Transfer-Encoding: chunked") + path.read_bytes()))
    return cases


class Server:
    def __init__(self, payloads, port):
        self.payloads = payloads
        self.port = port
        self.loop = asyncio.new_event_loop()
        self.ready = threading.Event()

    async def answer(self, reader, writer):
        try:
            request = await asyncio.wait_for(reader.readuntil(b"\r\n\r\n"), 5)
            case = int(request.split(b" ", 2)[1].rsplit(b"/", 1)[1])
            writer.write(self.payloads[case])
            await writer.drain()
            writer.write_eof()
            await asyncio.wait_for(reader.read(), 5)
        except (asyncio.TimeoutError, asyncio.IncompleteReadError, OSError, ValueError, IndexError):
            pass
        finally:
            writer.close()

    def serve(self):
        asyncio.set_event_loop(self.loop)
        started = asyncio.start_server(self.answer, "127.0.0.1", self.port, backlog=4096, limit=1 << 20)
        self.loop.run_until_complete(started)
        self.ready.set()
        self.loop.run_forever()

    def start(self):
        threading.Thread(target=self.serve, daemon=True).start()
        self.ready.wait()


def curl_reply(port, case):
    command = ["curl", "-sS", "--http1.1", "--max-time", "3", "-o", "-", "-w", "%{stderr}%{http_code}", f"http://127.0.0.1:{port}/c/{case}"]
    result = subprocess.run(command, stdin=subprocess.DEVNULL, capture_output=True)
    if result.returncode:
        return {"case": case, "error": f"curl {result.returncode}"}
    return {"case": case, "status": int(result.stderr.decode()[-3:]), "body": result.stdout.hex()}


def run_curl(port, count):
    with concurrent.futures.ThreadPoolExecutor(32) as pool:
        return list(pool.map(lambda case: curl_reply(port, case), range(count)))


def run_process(command):
    result = subprocess.run(command, stdin=subprocess.DEVNULL, capture_output=True, check=True)
    return [json.loads(row) for row in result.stdout.splitlines()]


def signature(outcome):
    if "error" in outcome:
        return REJECT
    return (outcome["status"], bytes.fromhex(outcome["body"]))


def preview(raw):
    return raw[:160].decode("latin-1").encode("unicode_escape").decode()


def partition(signatures):
    groups = collections.defaultdict(list)
    for client, value in signatures.items():
        groups[value].append(client)
    return " | ".join(sorted("+".join(sorted(members)) for members in groups.values()))


def table_report(outcomes, count):
    wrong = collections.defaultdict(list)
    for case in range(count):
        name, _, allowed = TABLE[case]
        for client, rows in outcomes.items():
            value = signature(rows[case])
            if value not in allowed:
                wrong[client].append({"case": name, "got": "reject" if value == REJECT else f"{value[0]} {preview(value[1])}", "detail": rows[case].get("error", "")})
    return {client: wrong.get(client, []) for client in outcomes}


def read_differently(signatures):
    accepted = [value for value in signatures.values() if value != REJECT]
    return len(set(accepted)) > 1


def disagreement_report(cases, outcomes, first):
    counts = collections.Counter()
    examples = collections.defaultdict(list)
    for case in range(first, len(cases)):
        signatures = {client: signature(rows[case]) for client, rows in outcomes.items()}
        if len(set(signatures.values())) == 1:
            continue
        if read_differently(signatures):
            counts["two clients accept and read different messages"] += 1
        key = partition(signatures)
        if read_differently(signatures):
            key = "framing: " + key
        counts[key] += 1
        if len(examples[key]) < 5:
            shown = {c: ("reject " + outcomes[c][case].get("error", "")[:80]) if s == REJECT else f"{s[0]} {preview(s[1])[:60]}" for c, s in signatures.items()}
            examples[key].append({"name": cases[case][0], "input": preview(cases[case][1]), "replies": shown})
    return [{"split": key, "count": count, "examples": examples[key]} for key, count in counts.most_common()]


def main():
    parser = argparse.ArgumentParser(description="Send the same response bytes to several HTTP clients and compare what each one reads")
    parser.add_argument("--qualify", default=str(HERE / "target/release/qualify"))
    parser.add_argument("--go-client", help="Go client binary; skipped when absent")
    parser.add_argument("--corpus", help="fuzz corpus folder holding h1_head and h1_chunked")
    parser.add_argument("--port", type=int, default=19000)
    parser.add_argument("--out", default=str(HERE / "results/differential.json"))
    args = parser.parse_args()
    cases = [(name, raw) for name, raw, _ in TABLE] + corpus_cases(args.corpus)
    server = Server([raw for _, raw in cases], args.port)
    server.start()
    count = str(len(cases))
    flags = ["--port", str(args.port), "--cases", count]
    outcomes = {
        "leyline": run_process([args.qualify, "differential", "--client", "leyline", *flags]),
        "hyper": run_process([args.qualify, "differential", "--client", "hyper", *flags]),
        "curl": run_curl(args.port, len(cases)),
    }
    if args.go_client:
        outcomes["go"] = run_process([args.go_client, "-port", str(args.port), "-cases", count])
    indexed = {client: {row["case"]: row for row in rows} for client, rows in outcomes.items()}
    table = table_report(indexed, len(TABLE))
    splits = disagreement_report(cases, indexed, len(TABLE))
    report = {"cases": len(cases), "table_cases": len(TABLE), "against_rfc": table, "splits": splits}
    pathlib.Path(args.out).parent.mkdir(parents=True, exist_ok=True)
    pathlib.Path(args.out).write_text(json.dumps(report, indent=1))
    print(f"cases {len(cases)}, table {len(TABLE)}")
    for client, wrong in table.items():
        print(f"{client}: {len(wrong)} table cases outside the RFC")
        for row in wrong:
            print(f"   {row['case']}: {row['got'][:70]} {row['detail'][:80]}")
    for split in splits:
        print(f"{split['count']:6d}  {split['split']}")
    return 1 if table["leyline"] else 0


if __name__ == "__main__":
    raise SystemExit(main())
