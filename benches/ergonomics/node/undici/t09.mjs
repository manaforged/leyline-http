import { ProxyAgent, request } from "undici";

const dispatcher = new ProxyAgent("http://127.0.0.1:18743");
const { statusCode, body } = await request(
  "http://ergonomics.invalid/t09/hello",
  { dispatcher },
);
const result = await body.json();
console.log(statusCode, result.via);
await dispatcher.close();
