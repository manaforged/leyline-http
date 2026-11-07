import { post } from "leyline-http";

const response = await post("http://127.0.0.1:18743/t07/upload", {
  form: { note: "hello" },
  files: { file: "upload.bin" },
});
const result = response.json();
console.log(response.status, result.size, result.note);
