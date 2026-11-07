import got, { TimeoutError } from "got";

const json = { name: "ada", age: 36 };
const response = await got.post("http://127.0.0.1:18743/t02/users", {
  json,
  timeout: { request: 5000 },
  responseType: "json",
});
console.log(response.statusCode, response.body.id);
try {
  await got.post("http://127.0.0.1:18743/t02/slow", {
    json,
    timeout: { request: 300 },
  });
  console.log("no timeout");
} catch (error) {
  if (!(error instanceof TimeoutError)) throw error;
  console.log("timeout");
}
