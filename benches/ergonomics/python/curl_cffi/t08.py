import time

import curl_cffi

url = "http://127.0.0.1:18743/t08/events"
session = curl_cffi.Session()
last_id = None
done = False
while not done:
    headers = {"Last-Event-ID": last_id} if last_id else {}
    response = session.get(url, headers=headers, stream=True)
    fields = {}
    for line in response.iter_lines():
        line = line.decode()
        if line:
            name, _, value = line.partition(": ")
            fields[name] = value
            continue
        if fields.get("event") == "done":
            done = True
            break
        last_id = fields.get("id", last_id)
        if "data" in fields:
            print(fields["data"])
        fields = {}
    response.close()
    time.sleep(0.1)
