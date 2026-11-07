import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { caPath, requestHeaders, run } from './harness.mjs';

const require = createRequire(import.meta.url);
const { Session, profiles } = require('leyline-http');
if (process.argv[2] === 'profiles') {
  console.log(profiles().filter((p) => p.startsWith('chrome-')).map((p) => p.slice('chrome-'.length)).join('\n'));
  process.exit(0);
}

const chrome = process.env.CMP_CHROME;
if (chrome && !/^\d+$/.test(chrome)) throw new Error(`leyline-node: CMP_CHROME must be a version number, got ${chrome}`);
if ((process.env.CMP_TIMEOUT ?? 'default') !== 'default') throw new Error('leyline-node supports only CMP_TIMEOUT=default');
const protocol = process.env.CMP_PROTO ?? 'h2';
if (!['h1', 'h2'].includes(protocol)) throw new Error('leyline-node supports CMP_PROTO=h1 or h2');
const certificate = caPath();
const headers = requestHeaders();

await run('leyline-node', {
  open: () => new Session({
    browser: chrome ? `chrome${chrome}` : 'none',
    protocol,
    trustEnv: false,
    verify: Boolean(certificate),
    caFiles: certificate ? [certificate] : undefined,
    headers,
  }),
  get: async (client, url) => {
    const response = await client.get(url);
    assert.equal(response.httpVersion, protocol === 'h1' ? 'HTTP/1.1' : 'HTTP/2');
    return { status: response.status, body: response.bytes() };
  },
  close: (client) => client.close(),
});
