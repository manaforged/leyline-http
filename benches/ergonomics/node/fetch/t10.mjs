async function follow(url, cap) {
  for (let hops = 0; hops <= cap; hops++) {
    const response = await fetch(url, { redirect: "manual" });
    await response.arrayBuffer();
    if (response.status < 300 || response.status >= 400) return url;
    url = new URL(response.headers.get("location"), url).href;
  }
  return null;
}
console.log(await follow("http://127.0.0.1:18743/t10/start", 5));
const looped = await follow("http://127.0.0.1:18743/t10/loop/0", 5);
console.log(looped === null ? "capped" : "uncapped");
