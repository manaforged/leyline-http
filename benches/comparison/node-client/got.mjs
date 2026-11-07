import http2 from 'node:http2';
import https from 'node:https';
import got from 'got';
import { caPem, requestHeaders, run } from './harness.mjs';

const ca = caPem();
const h1 = process.env.CMP_PROTO === 'h1';
const headers = requestHeaders();
const concC = Number.parseInt(process.argv[6] ?? '', 10) || 64;
const wire = h1 ? '1.1' : '2.0';
const common = { headers, retry: { limit: 0 }, responseType: 'buffer', throwHttpErrors: false };

const openH1 = () => {
  const agent = new https.Agent({ keepAlive: true, maxSockets: concC, maxFreeSockets: concC });
  const tls = ca ? { certificateAuthority: ca } : { rejectUnauthorized: false };
  return {
    client: got.extend({ ...common, agent: { https: agent }, https: tls }),
    close: async () => agent.destroy(),
  };
};

const openH2 = (url) => {
  const session = http2.connect(new URL(url).origin, ca ? { ca } : { rejectUnauthorized: false });
  return {
    client: got.extend({ ...common, http2: true, h2session: session }),
    close: () => new Promise((resolve) => session.close(resolve)),
  };
};

await run('got', {
  open: h1 ? openH1 : openH2,
  get: async ({ client }, url) => {
    const res = await client(url);
    if (res.httpVersion !== wire) throw new Error(`HTTP version ${res.httpVersion}`);
    return { status: res.statusCode, body: res.body };
  },
  close: ({ close }) => close(),
});
