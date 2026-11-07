import { Client, Pool } from 'undici';
import { caPem, run } from './harness.mjs';

const ca = caPem();
const tls = ca ? { ca } : { rejectUnauthorized: false };
const h1 = process.env.CMP_PROTO === 'h1';
const concC = Number.parseInt(process.argv[6] ?? '', 10) || 64;

const open = (url) => {
  const origin = new URL(url).origin;
  if (h1) return new Pool(origin, { allowH2: false, connections: concC, connect: tls });
  return new Client(origin, { allowH2: true, connect: tls });
};

await run('undici', {
  open,
  get: async (client, url) => {
    const u = new URL(url);
    const { statusCode, body } = await client.request({ path: u.pathname + u.search, method: 'GET' });
    return { status: statusCode, body: Buffer.from(await body.arrayBuffer()) };
  },
  close: (client) => client.close(),
});
