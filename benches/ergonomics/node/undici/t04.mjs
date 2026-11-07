import { Agent, RetryAgent, request } from "undici";

const dispatcher = new RetryAgent(new Agent(), {
  maxRetries: 2,
  minTimeout: 100,
  statusCodes: [503],
});
const { statusCode, body } = await request("http://127.0.0.1:18743/t04/flaky", {
  dispatcher,
});
const result = await body.json();
console.log(statusCode, result.attempts);
await dispatcher.close();
