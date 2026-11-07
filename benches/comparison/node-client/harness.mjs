import { readFileSync } from 'node:fs';
import { X509Certificate } from 'node:crypto';

export function caPem() {
  const path = process.env.CMP_CA;
  if (!path) return null;
  return new X509Certificate(readFileSync(path)).toString();
}

function expectedBody() {
  const size = process.env.CMP_BODY;
  if (size === undefined) return Buffer.from('ok-10byte!');
  const n = Number.parseInt(size, 10);
  if (!Number.isFinite(n)) throw new Error('CMP_BODY is a byte count');
  const out = Buffer.alloc(n);
  for (let i = 0; i < n; i++) out[i] = i % 251;
  return out;
}

function fnv1a(bytes) {
  let h = 0xcbf29ce484222325n;
  for (const b of bytes) {
    h ^= BigInt(b);
    h = (h * 0x100000001b3n) & 0xffffffffffffffffn;
  }
  return h.toString(16).padStart(16, '0');
}

function arg(args, i, fallback) {
  const v = Number.parseInt(args[i] ?? '', 10);
  return Number.isFinite(v) ? v : fallback;
}

function pct(sorted, p) {
  return sorted[Math.round((sorted.length - 1) * p)];
}

function micros(t0) {
  return Number((process.hrtime.bigint() - t0) / 1000n);
}

export async function run(name, { open, get, close }) {
  const args = process.argv.slice(2);
  const url = args[0] ?? 'https://127.0.0.1:8443/';
  const expected = expectedBody();

  const check = ({ status, body }) => {
    if (status !== 200) throw new Error(`status ${status}`);
    if (!expected.equals(body)) throw new Error('response body mismatch');
  };

  if (args[1] === 'equiv') {
    const client = open(url);
    const { status, body } = await get(client, url);
    console.log(`EQUIV ${name} status=${status} fnv=${fnv1a(body)} len=${body.length}`);
    await close(client);
    return;
  }

  const warmN = arg(args, 1, 2000);
  const coldN = arg(args, 2, 200);
  const concN = arg(args, 3, 20000);
  const concC = arg(args, 4, 64);
  const connections = Math.max(1, arg([process.env.CMP_CONNECTIONS], 0, 1));

  const clients = Array.from({ length: connections }, () => open(url));
  for (const c of clients) check(await get(c, url));

  const warmUs = new Array(warmN);
  let start = process.hrtime.bigint();
  for (let i = 0; i < warmN; i++) {
    const t0 = process.hrtime.bigint();
    check(await get(clients[0], url));
    warmUs[i] = micros(t0);
  }
  const warmRps = warmN / (micros(start) / 1e6);
  warmUs.sort((a, b) => a - b);

  const per = Math.floor(concN / concC);
  start = process.hrtime.bigint();
  const workers = Array.from({ length: concC }, async (_, index) => {
    const c = clients[index % connections];
    const lat = new Array(per);
    for (let i = 0; i < per; i++) {
      const t0 = process.hrtime.bigint();
      check(await get(c, url));
      lat[i] = micros(t0);
    }
    return lat;
  });
  const concUs = (await Promise.all(workers)).flat();
  const concRps = (per * concC) / (micros(start) / 1e6);
  concUs.sort((a, b) => a - b);

  start = process.hrtime.bigint();
  for (let i = 0; i < coldN; i++) {
    const fresh = open(url);
    check(await get(fresh, url));
    await close(fresh);
  }
  const coldRps = coldN / (micros(start) / 1e6);

  await Promise.all(clients.map(close));
  console.log(
    `RESULT ${name} connections=${connections} warm_rps=${warmRps.toFixed(3)} conc_rps=${concRps.toFixed(3)} cold_rps=${coldRps.toFixed(3)}` +
      ` warm_p50_us=${pct(warmUs, 0.5)} warm_p90_us=${pct(warmUs, 0.9)} warm_p99_us=${pct(warmUs, 0.99)}` +
      ` conc_p50_us=${pct(concUs, 0.5)} conc_p90_us=${pct(concUs, 0.9)} conc_p99_us=${pct(concUs, 0.99)} conc_p999_us=${pct(concUs, 0.999)}`,
  );
}
