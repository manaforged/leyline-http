import time

import niquests

url = "psse://127.0.0.1:18743/t08/events"
session = niquests.Session()
last_id = None
done = False
while not done:
    headers = {"Last-Event-ID": last_id} if last_id else {}
    response = session.get(url, headers=headers)
    while not done and (event := response.extension.next_payload()):
        if event.event == "done":
            done = True
        else:
            last_id = event.id or last_id
            if event.data:
                print(event.data)
    time.sleep(0.1)
