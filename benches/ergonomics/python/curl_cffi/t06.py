import curl_cffi

url = "http://127.0.0.1:18743/t06/large"
session = curl_cffi.Session()
total = int(session.head(url).headers["Content-Length"])
with open("large.bin", "ab") as file:
    while file.tell() < total:
        headers = {"Range": f"bytes={file.tell()}-"}
        try:
            session.get(url, headers=headers, content_callback=file.write)
        except curl_cffi.exceptions.RequestException:
            pass
    assert file.tell() == total
    print(file.tell())
