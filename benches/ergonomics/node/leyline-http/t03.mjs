import { Session } from "leyline-http";

const session = new Session();
await session.post("http://localhost:18743/t03/login", {
  json: { user: "ada", password: "secret" },
});
const response = await session.get("http://localhost:18743/t03/me");
console.log(response.json().user);
