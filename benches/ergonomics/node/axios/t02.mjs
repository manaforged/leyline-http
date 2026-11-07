import axios from "axios";

const body = { name: "ada", age: 36 };
const { status, data } = await axios.post(
  "http://127.0.0.1:18743/t02/users",
  body,
  {
    timeout: 5000,
  },
);
console.log(status, data.id);
try {
  await axios.post("http://127.0.0.1:18743/t02/slow", body, { timeout: 300 });
  console.log("no timeout");
} catch (error) {
  if (error.code !== "ECONNABORTED") throw error;
  console.log("timeout");
}
