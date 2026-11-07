const response = await fetch("http://127.0.0.1:18743/t01/user");
const user = await response.json();
console.log(user.id, user.name);
