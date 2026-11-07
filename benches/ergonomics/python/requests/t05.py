from concurrent.futures import ThreadPoolExecutor

import requests


def fetch(i):
    return requests.get(f"http://127.0.0.1:18743/t05/item/{i}").status_code


with ThreadPoolExecutor(max_workers=10) as pool:
    statuses = list(pool.map(fetch, range(100)))
print(statuses.count(200), statuses.count(404))
