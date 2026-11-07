import asyncio

import aiohttp


async def main():
    async with aiohttp.ClientSession() as session:
        async with session.get(
            "http://ergonomics.invalid/t09/hello", proxy="http://127.0.0.1:18743"
        ) as response:
            body = await response.json()
            print(response.status, body["via"])


asyncio.run(main())
