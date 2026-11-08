import { openAsBlob } from "node:fs";
import got from "got";

const form = new FormData();
form.append("note", "hello");
form.append("file", await openAsBlob("upload.bin"), "upload.bin");
const response = await got.post("http://127.0.0.1:18743/t07/upload", {
  body: form,
  responseType: "json",
});
console.log(response.statusCode, response.body.size, response.body.note);
