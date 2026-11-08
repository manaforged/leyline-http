import axios from "axios";
import { wrapper } from "axios-cookiejar-support";
import { CookieJar } from "tough-cookie";

const client = wrapper(axios.create({ jar: new CookieJar() }));
await client.post("http://localhost:18743/t03/login", {
  user: "ada",
  password: "secret",
});
const { data: me } = await client.get("http://localhost:18743/t03/me");
console.log(me.user);
