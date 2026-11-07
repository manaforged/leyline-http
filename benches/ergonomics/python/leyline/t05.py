import asyncio

from leyline import AsyncSession


async def main():
    session = AsyncSession(max_in_flight=10)
    responses = await asyncio.gather(
        *(session.get(f"http://127.0.0.1:18743/t05/item/{i}") for i in range(100))
    )
    statuses = [response.status for response in responses]
    print(statuses.count(200), statuses.count(404))


asyncio.run(main())
