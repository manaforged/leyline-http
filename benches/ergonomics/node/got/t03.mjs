import got from "got";
import { CookieJar } from "tough-cookie";

const client = got.extend({ cookieJar: new CookieJar() });
await client.post("http://localhost:18743/t03/login", {
  json: { user: "ada", password: "secret" },
});
const me = await client("http://localhost:18743/t03/me").json();
console.log(me.user);
