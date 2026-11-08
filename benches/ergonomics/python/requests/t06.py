import requests

url = "http://127.0.0.1:18743/t06/large"
total = int(requests.head(url).headers["Content-Length"])
with open("large.bin", "ab") as file:
    while file.tell() < total:
        headers = {"Range": f"bytes={file.tell()}-"}
        try:
            with requests.get(url, headers=headers, stream=True) as response:
                for chunk in response.iter_content(65536):
                    file.write(chunk)
        except requests.RequestException:
            pass
    assert file.tell() == total
    print(file.tell())
