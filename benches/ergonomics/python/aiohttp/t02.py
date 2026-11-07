import asyncio

import aiohttp

body = {"name": "ada", "age": 36}


async def main():
    async with aiohttp.ClientSession() as session:
        timeout = aiohttp.ClientTimeout(total=5)
        async with session.post(
            "http://127.0.0.1:18743/t02/users", json=body, timeout=timeout
        ) as response:
            created = await response.json()
            print(response.status, created["id"])
        try:
            timeout = aiohttp.ClientTimeout(total=0.3)
            async with session.post(
                "http://127.0.0.1:18743/t02/slow", json=body, timeout=timeout
            ):
                print("no timeout")
        except asyncio.TimeoutError:
            print("timeout")


asyncio.run(main())
