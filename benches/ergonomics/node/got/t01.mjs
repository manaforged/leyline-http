import got from "got";

const user = await got("http://127.0.0.1:18743/t01/user").json();
console.log(user.id, user.name);
