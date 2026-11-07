const request = {
  method: "POST",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify({ name: "ada", age: 36 }),
};
const response = await fetch("http://127.0.0.1:18743/t02/users", {
  ...request,
  signal: AbortSignal.timeout(5000),
});
const created = await response.json();
console.log(response.status, created.id);
try {
  await fetch("http://127.0.0.1:18743/t02/slow", {
    ...request,
    signal: AbortSignal.timeout(300),
  });
  console.log("no timeout");
} catch (error) {
  if (error.name !== "TimeoutError") throw error;
  console.log("timeout");
}
