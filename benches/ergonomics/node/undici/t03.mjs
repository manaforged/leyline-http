import { request } from "undici";

const login = await request("http://localhost:18743/t03/login", {
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify({ user: "ada", password: "secret" }),
});
await login.body.dump();
const cookie = login.headers["set-cookie"].split(";")[0];
const { body } = await request("http://localhost:18743/t03/me", {
  headers: { cookie },
});
const me = await body.json();
console.log(me.user);
