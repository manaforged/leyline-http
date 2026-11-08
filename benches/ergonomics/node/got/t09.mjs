import got from "got";
import { HttpProxyAgent } from "hpagent";

const response = await got("http://ergonomics.invalid/t09/hello", {
  agent: { http: new HttpProxyAgent({ proxy: "http://127.0.0.1:18743" }) },
  responseType: "json",
});
console.log(response.statusCode, response.body.via);
