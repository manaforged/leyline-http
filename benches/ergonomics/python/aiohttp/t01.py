import asyncio

import aiohttp


async def main():
    async with aiohttp.ClientSession() as session:
        async with session.get("http://127.0.0.1:18743/t01/user") as response:
            user = await response.json()
    print(user["id"], user["name"])


asyncio.run(main())
