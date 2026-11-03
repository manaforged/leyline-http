import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { caPem, run } from './harness.mjs';

const require = createRequire(import.meta.url);
const { Session, profiles } = require('leyline-http');
if (process.argv[2] === 'profiles') {
  console.log(profiles().filter((p) => p.startsWith('chrome-')).map((p) => p.slice('chrome-'.length)).join('\n'));
  process.exit(0);
}

const chrome = process.env.CMP_CHROME;
if (!chrome || !/^\d+$/.test(chrome)) throw new Error('leyline-node requires an explicit CMP_CHROME browser profile');
if ((process.env.CMP_TIMEOUT ?? 'default') !== 'default') throw new Error('leyline-node supports only CMP_TIMEOUT=default');
const protocol = process.env.CMP_PROTO ?? 'h2';
if (!['h1', 'h2'].includes(protocol)) throw new Error('leyline-node supports CMP_PROTO=h1 or h2');
const ca = caPem();
const directory = ca ? mkdtempSync(join(tmpdir(), 'leyline-comparison-')) : undefined;
const certificate = directory ? join(directory, 'ca.pem') : undefined;

try {
  if (certificate) writeFileSync(certificate, ca);
  await run('leyline-node', {
    open: () => new Session({
      browser: `chrome${chrome}`,
      protocol,
      trustEnv: false,
      verify: Boolean(ca),
      caFiles: certificate ? [certificate] : undefined,
    }),
    get: async (client, url) => {
      const response = await client.get(url);
      assert.equal(response.httpVersion, protocol === 'h1' ? 'HTTP/1.1' : 'HTTP/2');
      return { status: response.status, body: response.bytes() };
    },
    close: (client) => client.close(),
  });
} finally {
  if (directory) rmSync(directory, { recursive: true });
}
