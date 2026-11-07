import asyncio

from aiohttp_sse_client2 import client


async def main():
    async with client.EventSource("http://127.0.0.1:18743/t08/events") as source:
        async for event in source:
            if event.type == "done":
                break
            print(event.data)


asyncio.run(main())
