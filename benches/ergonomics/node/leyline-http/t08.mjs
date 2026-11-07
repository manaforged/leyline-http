import { Session } from "leyline-http";

for await (const event of new Session().events("http://127.0.0.1:18743/t08/events")) {
  if (event.event === "done") break;
  console.log(event.data);
}
