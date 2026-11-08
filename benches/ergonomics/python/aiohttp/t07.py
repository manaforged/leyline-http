import asyncio

import aiohttp


async def main():
    async with aiohttp.ClientSession() as session:
        with open("upload.bin", "rb") as file:
            data = {"note": "hello", "file": file}
            async with session.post(
                "http://127.0.0.1:18743/t07/upload", data=data
            ) as response:
                body = await response.json()
                print(response.status, body["size"], body["note"])


asyncio.run(main())
