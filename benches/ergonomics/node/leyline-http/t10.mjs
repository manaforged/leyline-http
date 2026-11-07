import { Session } from "leyline-http";

const session = new Session({ redirect: 5 });
const response = await session.get("http://127.0.0.1:18743/t10/start");
console.log(response.url);
const looped = await session.get("http://127.0.0.1:18743/t10/loop/0");
console.log(looped.status === 302 ? "capped" : "uncapped");
