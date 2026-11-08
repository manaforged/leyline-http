import { createWriteStream } from "node:fs";
import { pipeline } from "node:stream/promises";
import got from "got";

const url = "http://127.0.0.1:18743/t06/large";
const head = await got.head(url);
const total = Number(head.headers["content-length"]);
const file = createWriteStream("large.bin");
while (file.bytesWritten + file.writableLength < total) {
  const headers = {
    range: `bytes=${file.bytesWritten + file.writableLength}-`,
  };
  await pipeline(got.stream(url, { headers, retry: { limit: 0 } }), file, {
    end: false,
  }).catch(() => {});
}
await new Promise((resolve) => file.end(resolve));
if (file.bytesWritten !== total) throw new Error("length mismatch");
console.log(file.bytesWritten);
