import http2 from 'node:http2';
import { caPem, requestHeaders, run } from './harness.mjs';

if (process.env.CMP_PROTO === 'h1') {
  console.error('node-http2 speaks only HTTP/2; unset CMP_PROTO or set it to h2');
  process.exit(1);
}

const ca = caPem();
const tls = ca ? { ca } : { rejectUnauthorized: false };
const headers = requestHeaders();

await run('node-http2', {
  open: (url) => http2.connect(new URL(url).origin, tls),
  get: (session, url) =>
    new Promise((resolve, reject) => {
      const u = new URL(url);
      const req = session.request({ ':method': 'GET', ':path': u.pathname + u.search, ...headers });
      let status = 0;
      const chunks = [];
      req.on('response', (headers) => {
        status = headers[':status'];
      });
      req.on('data', (chunk) => chunks.push(chunk));
      req.on('end', () => resolve({ status, body: Buffer.concat(chunks) }));
      req.on('error', reject);
      req.end();
    }),
  close: (session) => new Promise((resolve) => session.close(resolve)),
});
