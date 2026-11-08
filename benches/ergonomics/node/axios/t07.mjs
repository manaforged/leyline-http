import { createReadStream } from "node:fs";
import axios from "axios";

const { status, data } = await axios.postForm(
  "http://127.0.0.1:18743/t07/upload",
  {
    note: "hello",
    file: createReadStream("upload.bin"),
  },
);
console.log(status, data.size, data.note);
