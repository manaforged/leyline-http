import { Agent, request } from "undici";

const dispatcher = new Agent({ connections: 10 });
const statuses = await Promise.all(
  Array.from({ length: 100 }, async (_, i) => {
    const { statusCode, body } = await request(
      "http://127.0.0.1:18743/t05/item/" + i,
      {
        dispatcher,
      },
    );
    await body.dump();
    return statusCode;
  }),
);
const count = (status) => statuses.filter((s) => s === status).length;
console.log(count(200), count(404));
await dispatcher.close();
