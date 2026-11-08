import axios from "axios";

const { data: user } = await axios.get("http://127.0.0.1:18743/t01/user");
console.log(user.id, user.name);
