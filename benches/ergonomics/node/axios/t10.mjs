import axios from "axios";

const response = await axios.get("http://127.0.0.1:18743/t10/start", {
  maxRedirects: 5,
});
console.log(response.request.res.responseUrl);
try {
  await axios.get("http://127.0.0.1:18743/t10/loop/0", { maxRedirects: 5 });
  console.log("uncapped");
} catch (error) {
  if (error.code !== "ERR_FR_TOO_MANY_REDIRECTS") throw error;
  console.log("capped");
}
