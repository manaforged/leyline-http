import httpx
from tenacity import (
    Retrying,
    retry_if_result,
    stop_after_attempt,
    wait_exponential,
)

backoff = wait_exponential(multiplier=0.1)


def wait(state):
    retry_after = state.outcome.result().headers.get("Retry-After")
    return float(retry_after) if retry_after else backoff(state)


retrying = Retrying(
    retry=retry_if_result(lambda response: response.status_code == 503),
    stop=stop_after_attempt(3),
    wait=wait,
)
response = retrying(httpx.get, "http://127.0.0.1:18743/t04/flaky")
print(response.status_code, response.json()["attempts"])
