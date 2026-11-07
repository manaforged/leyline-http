import { Session } from "leyline-http";

const session = new Session({ maxInFlight: 10 });
const responses = await Promise.all(
  Array.from({ length: 100 }, (_, i) =>
    session.get("http://127.0.0.1:18743/t05/item/" + i),
  ),
);
const count = (status) => responses.filter((r) => r.status === status).length;
console.log(count(200), count(404));
