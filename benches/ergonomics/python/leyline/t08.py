from leyline import Session

for event in Session().events("http://127.0.0.1:18743/t08/events"):
    if event.event == "done":
        break
    print(event.data)
