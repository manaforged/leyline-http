import asyncio

import curl_cffi


async def main():
    async with curl_cffi.AsyncSession(max_clients=10) as session:
        responses = await asyncio.gather(
            *(session.get(f"http://127.0.0.1:18743/t05/item/{i}") for i in range(100))
        )
    statuses = [response.status_code for response in responses]
    print(statuses.count(200), statuses.count(404))


asyncio.run(main())
