import { createWriteStream } from "node:fs";
import { pipeline } from "node:stream/promises";
import axios from "axios";

const url = "http://127.0.0.1:18743/t06/large";
const head = await axios.head(url);
const total = Number(head.headers["content-length"]);
const file = createWriteStream("large.bin");
while (file.bytesWritten + file.writableLength < total) {
  const headers = {
    Range: `bytes=${file.bytesWritten + file.writableLength}-`,
  };
  const { data } = await axios.get(url, { headers, responseType: "stream" });
  await pipeline(data, file, { end: false }).catch(() => {});
}
await new Promise((resolve) => file.end(resolve));
if (file.bytesWritten !== total) throw new Error("length mismatch");
console.log(file.bytesWritten);
