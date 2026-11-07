import https from 'node:https';
import axios from 'axios';
import { caPem, requestHeaders, run } from './harness.mjs';

if (process.env.CMP_PROTO !== 'h1') {
  console.error('axios speaks only HTTP/1.1; set CMP_PROTO=h1');
  process.exit(1);
}

const ca = caPem();
const tls = ca ? { ca } : { rejectUnauthorized: false };
const headers = requestHeaders();
const concC = Number.parseInt(process.argv[6] ?? '', 10) || 64;

await run('axios', {
  open: () => {
    const httpsAgent = new https.Agent({ keepAlive: true, maxSockets: concC, maxFreeSockets: concC, ...tls });
    return axios.create({ httpsAgent, headers, responseType: 'arraybuffer', validateStatus: () => true });
  },
  get: async (client, url) => {
    const res = await client.get(url);
    const version = res.request?.res?.httpVersion;
    if (version !== '1.1') throw new Error(`HTTP version ${version}`);
    return { status: res.status, body: Buffer.from(res.data) };
  },
  close: (client) => client.defaults.httpsAgent.destroy(),
});
