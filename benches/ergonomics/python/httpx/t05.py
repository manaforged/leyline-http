import asyncio

import httpx


async def main():
    limits = httpx.Limits(max_connections=10)
    async with httpx.AsyncClient(limits=limits, timeout=30) as client:
        responses = await asyncio.gather(
            *(client.get(f"http://127.0.0.1:18743/t05/item/{i}") for i in range(100))
        )
    statuses = [response.status_code for response in responses]
    print(statuses.count(200), statuses.count(404))


asyncio.run(main())
