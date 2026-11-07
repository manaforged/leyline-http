import pLimit from "p-limit";

const limit = pLimit(10);
const statuses = await Promise.all(
  Array.from({ length: 100 }, (_, i) =>
    limit(async () => {
      const response = await fetch("http://127.0.0.1:18743/t05/item/" + i);
      await response.arrayBuffer();
      return response.status;
    }),
  ),
);
const count = (status) => statuses.filter((s) => s === status).length;
console.log(count(200), count(404));
