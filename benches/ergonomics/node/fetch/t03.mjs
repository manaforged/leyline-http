const login = await fetch("http://localhost:18743/t03/login", {
  method: "POST",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify({ user: "ada", password: "secret" }),
});
const cookie = login.headers.getSetCookie()[0].split(";")[0];
const response = await fetch("http://localhost:18743/t03/me", {
  headers: { cookie },
});
const me = await response.json();
console.log(me.user);
