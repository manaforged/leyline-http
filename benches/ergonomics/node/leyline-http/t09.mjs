import { get } from "leyline-http";

const response = await get("http://ergonomics.invalid/t09/hello", {
  proxy: "http://127.0.0.1:18743",
});
console.log(response.status, response.json().via);
