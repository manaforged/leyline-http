import asyncio

import aiohttp


async def main():
    async with aiohttp.ClientSession() as session:
        async with session.get(
            "http://127.0.0.1:18743/t10/start", max_redirects=5
        ) as response:
            print(response.url)
        try:
            async with session.get(
                "http://127.0.0.1:18743/t10/loop/0", max_redirects=5
            ):
                print("uncapped")
        except aiohttp.TooManyRedirects:
            print("capped")


asyncio.run(main())
