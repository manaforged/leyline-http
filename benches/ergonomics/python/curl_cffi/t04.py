import time

import curl_cffi

session = curl_cffi.Session()
url = "http://127.0.0.1:18743/t04/flaky"
response = session.get(url)
backoff = 0.1
for _ in range(2):
    if response.status_code != 503:
        break
    time.sleep(float(response.headers.get("Retry-After") or backoff))
    backoff *= 2
    response = session.get(url)
print(response.status_code, response.json()["attempts"])
