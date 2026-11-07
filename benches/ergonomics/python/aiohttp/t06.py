import asyncio

import aiohttp

url = "http://127.0.0.1:18743/t06/large"


async def main():
    async with aiohttp.ClientSession() as session:
        async with session.head(url) as response:
            total = response.content_length
        with open("large.bin", "ab") as file:
            while file.tell() < total:
                headers = {"Range": f"bytes={file.tell()}-"}
                try:
                    async with session.get(url, headers=headers) as response:
                        async for chunk in response.content.iter_chunked(65536):
                            file.write(chunk)
                except aiohttp.ClientError:
                    pass
            assert file.tell() == total
            print(file.tell())


asyncio.run(main())
