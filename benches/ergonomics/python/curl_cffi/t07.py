import curl_cffi

form = curl_cffi.CurlMime()
form.addpart(name="note", data=b"hello")
form.addpart(name="file", filename="upload.bin", local_path="upload.bin")
response = curl_cffi.post("http://127.0.0.1:18743/t07/upload", multipart=form)
form.close()
body = response.json()
print(response.status_code, body["size"], body["note"])
