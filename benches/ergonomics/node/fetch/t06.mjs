import { createWriteStream } from "node:fs";
import { pipeline } from "node:stream/promises";
import { Readable } from "node:stream";

const url = "http://127.0.0.1:18743/t06/large";
const head = await fetch(url, { method: "HEAD" });
const total = Number(head.headers.get("content-length"));
const file = createWriteStream("large.bin");
while (file.bytesWritten + file.writableLength < total) {
  const headers = {
    Range: `bytes=${file.bytesWritten + file.writableLength}-`,
  };
  const response = await fetch(url, { headers });
  await pipeline(Readable.fromWeb(response.body), file, { end: false }).catch(
    () => {},
  );
}
await new Promise((resolve) => file.end(resolve));
if (file.bytesWritten !== total) throw new Error("length mismatch");
console.log(file.bytesWritten);
