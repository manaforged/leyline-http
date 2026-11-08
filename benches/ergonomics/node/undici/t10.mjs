import { Agent, interceptors, request } from "undici";

const dispatcher = new Agent().compose(
  interceptors.redirect({ maxRedirections: 5 }),
);
const { body, context } = await request("http://127.0.0.1:18743/t10/start", {
  dispatcher,
});
await body.dump();
console.log(context.history.at(-1).href);
const looped = await request("http://127.0.0.1:18743/t10/loop/0", {
  dispatcher,
});
await looped.body.dump();
console.log(looped.statusCode === 302 ? "capped" : "uncapped");
await dispatcher.close();
