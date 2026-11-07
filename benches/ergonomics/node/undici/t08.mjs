import { EventSource } from "undici";

const alive = setInterval(() => {}, 1000);
const source = new EventSource("http://127.0.0.1:18743/t08/events");
source.onmessage = (event) => console.log(event.data);
source.addEventListener("done", () => {
  source.close();
  clearInterval(alive);
});
