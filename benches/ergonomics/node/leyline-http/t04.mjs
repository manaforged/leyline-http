import { Session } from "leyline-http";

const session = new Session({ retry: { maxRetries: 2 } });
const response = await session.get("http://127.0.0.1:18743/t04/flaky");
console.log(response.status, response.json().attempts);
