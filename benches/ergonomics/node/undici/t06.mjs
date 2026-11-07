import { createWriteStream } from "node:fs";
import { pipeline } from "node:stream/promises";
import { request } from "undici";

const url = "http://127.0.0.1:18743/t06/large";
const head = await request(url, { method: "HEAD" });
await head.body.dump();
const total = Number(head.headers["content-length"]);
const file = createWriteStream("large.bin");
while (file.bytesWritten + file.writableLength < total) {
  const headers = {
    range: `bytes=${file.bytesWritten + file.writableLength}-`,
  };
  const { body } = await request(url, { headers });
  await pipeline(body, file, { end: false }).catch(() => {});
}
await new Promise((resolve) => file.end(resolve));
if (file.bytesWritten !== total) throw new Error("length mismatch");
console.log(file.bytesWritten);
