import { Agent } from "node:http";
import got from "got";

const client = got.extend({
  agent: { http: new Agent({ maxSockets: 10 }) },
  throwHttpErrors: false,
});
const responses = await Promise.all(
  Array.from({ length: 100 }, (_, i) =>
    client("http://127.0.0.1:18743/t05/item/" + i),
  ),
);
const count = (status) =>
  responses.filter((r) => r.statusCode === status).length;
console.log(count(200), count(404));
