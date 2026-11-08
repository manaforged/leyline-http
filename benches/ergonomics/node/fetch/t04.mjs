import { setTimeout as sleep } from "node:timers/promises";

const url = "http://127.0.0.1:18743/t04/flaky";
let response = await fetch(url);
let backoff = 100;
for (let i = 0; i < 2 && response.status === 503; i++) {
  const retryAfter = response.headers.get("retry-after");
  await sleep(retryAfter ? retryAfter * 1000 : backoff);
  backoff *= 2;
  response = await fetch(url);
}
const body = await response.json();
console.log(response.status, body.attempts);
