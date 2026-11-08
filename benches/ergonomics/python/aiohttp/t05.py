import asyncio

import aiohttp


async def main():
    connector = aiohttp.TCPConnector(limit=10)
    async with aiohttp.ClientSession(connector=connector) as session:

        async def fetch(i):
            async with session.get(f"http://127.0.0.1:18743/t05/item/{i}") as response:
                return response.status

        statuses = await asyncio.gather(*(fetch(i) for i in range(100)))
    print(statuses.count(200), statuses.count(404))


asyncio.run(main())
