import asyncio

import niquests


async def main():
    limit = asyncio.Semaphore(10)
    async with niquests.AsyncSession() as session:

        async def fetch(i):
            async with limit:
                response = await session.get(f"http://127.0.0.1:18743/t05/item/{i}")
                return response.status_code

        statuses = await asyncio.gather(*(fetch(i) for i in range(100)))
    print(statuses.count(200), statuses.count(404))


asyncio.run(main())
