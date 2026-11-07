import leyline

response = leyline.post(
    "http://127.0.0.1:18743/t07/upload",
    form={"note": "hello"},
    files={"file": "upload.bin"},
)
body = response.json()
print(response.status, body["size"], body["note"])
