import { openAsBlob } from "node:fs";

const form = new FormData();
form.append("note", "hello");
form.append("file", await openAsBlob("upload.bin"), "upload.bin");
const response = await fetch("http://127.0.0.1:18743/t07/upload", {
  method: "POST",
  body: form,
});
const body = await response.json();
console.log(response.status, body.size, body.note);
