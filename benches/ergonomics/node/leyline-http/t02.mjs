import { post, TimeoutError } from "leyline-http";

const json = { name: "ada", age: 36 };
const response = await post("http://127.0.0.1:18743/t02/users", {
  json,
  timeoutMs: 5000,
});
console.log(response.status, response.json().id);
try {
  await post("http://127.0.0.1:18743/t02/slow", {
    json,
    timeoutMs: 300,
  });
  console.log("no timeout");
} catch (error) {
  if (!(error instanceof TimeoutError)) throw error;
  console.log("timeout");
}
