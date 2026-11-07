import { setTimeout as sleep } from "node:timers/promises";
import { createInterface } from "node:readline";
import axios from "axios";

const url = "http://127.0.0.1:18743/t08/events";
let lastId;
let done = false;
while (!done) {
  const headers = lastId ? { "Last-Event-ID": lastId } : {};
  const { data } = await axios.get(url, { headers, responseType: "stream" });
  let fields = {};
  for await (const line of createInterface({ input: data })) {
    if (line) {
      const [name, value] = line.split(": ");
      fields[name] = value;
      continue;
    }
    if (fields.event === "done") {
      done = true;
      break;
    }
    lastId = fields.id ?? lastId;
    if (fields.data) console.log(fields.data);
    fields = {};
  }
  await sleep(100);
}
