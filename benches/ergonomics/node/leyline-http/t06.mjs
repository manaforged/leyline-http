import { Session } from "leyline-http";

const session = new Session({ retry: { maxRetries: 2 } });
console.log(await session.download("http://127.0.0.1:18743/t06/large", "large.bin"));
