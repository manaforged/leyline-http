import asyncio

import aiohttp
from tenacity import (
    AsyncRetrying,
    retry_if_result,
    stop_after_attempt,
    wait_exponential,
)

backoff = wait_exponential(multiplier=0.1)


def wait(state):
    retry_after = state.outcome.result().headers.get("Retry-After")
    return float(retry_after) if retry_after else backoff(state)


async def main():
    async with aiohttp.ClientSession() as session:

        async def fetch():
            return await session.get("http://127.0.0.1:18743/t04/flaky")

        retrying = AsyncRetrying(
            retry=retry_if_result(lambda response: response.status == 503),
            stop=stop_after_attempt(3),
            wait=wait,
        )
        response = await retrying(fetch)
        body = await response.json()
        print(response.status, body["attempts"])


asyncio.run(main())
