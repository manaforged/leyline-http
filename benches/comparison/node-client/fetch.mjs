import { caPem, requestHeaders, run } from './harness.mjs';

if (process.env.CMP_PROTO !== 'h1') {
  console.error(`fetch: the undici ${process.versions.undici} built into Node ${process.version} runs concurrent HTTP/2 streams into an assertion failure and queues them one at a time otherwise; set CMP_PROTO=h1`);
  process.exit(1);
}

const ca = caPem();
const tls = ca ? { ca } : { rejectUnauthorized: false };
const headers = requestHeaders();
const concC = Number.parseInt(process.argv[6] ?? '', 10) || 64;

await fetch('data:,');
const Agent = globalThis[Symbol.for('undici.globalDispatcher.1')].constructor;

const stripForced = (dispatch) => (options, handler) => {
  const { 'accept-language': _language, 'sec-fetch-mode': _mode, ...kept } = options.headers;
  return dispatch({ ...options, headers: kept }, handler);
};

await run('fetch', {
  open: () => {
    const agent = new Agent({ connections: concC, connect: tls });
    return { agent, dispatcher: headers ? agent.compose(stripForced) : agent };
  },
  get: async ({ dispatcher }, url) => {
    const res = await fetch(url, { headers, dispatcher });
    return { status: res.status, body: Buffer.from(await res.arrayBuffer()) };
  },
  close: ({ agent }) => agent.close(),
});
