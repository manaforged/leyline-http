import httpx

url = "http://127.0.0.1:18743/t06/large"
total = int(httpx.head(url).headers["Content-Length"])
with open("large.bin", "ab") as file:
    while file.tell() < total:
        headers = {"Range": f"bytes={file.tell()}-"}
        try:
            with httpx.stream("GET", url, headers=headers) as response:
                for chunk in response.iter_bytes():
                    file.write(chunk)
        except httpx.TransportError:
            pass
    assert file.tell() == total
    print(file.tell())
