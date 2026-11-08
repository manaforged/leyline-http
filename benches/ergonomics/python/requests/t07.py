import requests

with open("upload.bin", "rb") as file:
    response = requests.post(
        "http://127.0.0.1:18743/t07/upload",
        files={"file": file},
        data={"note": "hello"},
    )
body = response.json()
print(response.status_code, body["size"], body["note"])
