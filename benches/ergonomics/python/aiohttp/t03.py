import asyncio

import aiohttp


async def main():
    async with aiohttp.ClientSession() as session:
        login = {"user": "ada", "password": "secret"}
        async with session.post("http://localhost:18743/t03/login", json=login):
            pass
        async with session.get("http://localhost:18743/t03/me") as response:
            me = await response.json()
    print(me["user"])


asyncio.run(main())
