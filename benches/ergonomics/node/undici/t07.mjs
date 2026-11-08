import { openAsBlob } from "node:fs";
import { FormData, request } from "undici";

const form = new FormData();
form.append("note", "hello");
form.append("file", await openAsBlob("upload.bin"), "upload.bin");
const { statusCode, body } = await request(
  "http://127.0.0.1:18743/t07/upload",
  {
    method: "POST",
    body: form,
  },
);
const result = await body.json();
console.log(statusCode, result.size, result.note);
