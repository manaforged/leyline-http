import got from "got";

const response = await got("http://127.0.0.1:18743/t04/flaky", {
  retry: { limit: 2 },
  responseType: "json",
});
console.log(response.statusCode, response.body.attempts);
