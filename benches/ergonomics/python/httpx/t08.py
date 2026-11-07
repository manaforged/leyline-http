import time

import httpx
from httpx_sse import connect_sse

url = "http://127.0.0.1:18743/t08/events"
last_id = None
done = False
with httpx.Client() as client:
    while not done:
        headers = {"Last-Event-ID": last_id} if last_id else {}
        with connect_sse(client, "GET", url, headers=headers) as source:
            for event in source.iter_sse():
                if event.event == "done":
                    done = True
                    break
                last_id = event.id or last_id
                if event.data:
                    print(event.data)
        time.sleep(0.1)
