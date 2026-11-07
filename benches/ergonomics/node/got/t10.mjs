import got, { MaxRedirectsError } from "got";

const client = got.extend({ maxRedirects: 5 });
console.log((await client("http://127.0.0.1:18743/t10/start")).url);
try {
  await client("http://127.0.0.1:18743/t10/loop/0");
  console.log("uncapped");
} catch (error) {
  if (!(error instanceof MaxRedirectsError)) throw error;
  console.log("capped");
}
