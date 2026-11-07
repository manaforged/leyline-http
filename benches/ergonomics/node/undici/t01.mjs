import { request } from "undici";

const { body } = await request("http://127.0.0.1:18743/t01/user");
const user = await body.json();
console.log(user.id, user.name);
