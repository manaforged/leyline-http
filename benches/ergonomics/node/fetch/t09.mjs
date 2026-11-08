import { ProxyAgent } from "undici6";

const dispatcher = new ProxyAgent("http://127.0.0.1:18743");
const response = await fetch("http://ergonomics.invalid/t09/hello", {
  dispatcher,
});
const body = await response.json();
console.log(response.status, body.via);
await dispatcher.close();
