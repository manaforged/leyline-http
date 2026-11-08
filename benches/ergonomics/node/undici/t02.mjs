import { errors, request } from "undici";

const options = {
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify({ name: "ada", age: 36 }),
};
const { statusCode, body } = await request("http://127.0.0.1:18743/t02/users", {
  ...options,
  headersTimeout: 5000,
  bodyTimeout: 5000,
});
const created = await body.json();
console.log(statusCode, created.id);
try {
  await request("http://127.0.0.1:18743/t02/slow", {
    ...options,
    headersTimeout: 300,
    bodyTimeout: 300,
  });
  console.log("no timeout");
} catch (error) {
  if (!(error instanceof errors.HeadersTimeoutError)) throw error;
  console.log("timeout");
}
