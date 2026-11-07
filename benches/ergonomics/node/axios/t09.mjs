import axios from "axios";

const { status, data } = await axios.get(
  "http://ergonomics.invalid/t09/hello",
  { proxy: { host: "127.0.0.1", port: 18743 } },
);
console.log(status, data.via);
