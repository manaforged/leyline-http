import { get } from "leyline-http";

const response = await get("http://127.0.0.1:18743/t01/user");
const user = response.json();
console.log(user.id, user.name);
