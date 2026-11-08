from requests_sse import EventSource

with EventSource("http://127.0.0.1:18743/t08/events") as source:
    for event in source:
        if event.type == "done":
            break
        print(event.data)
