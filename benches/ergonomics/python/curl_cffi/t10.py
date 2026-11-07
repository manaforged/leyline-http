import curl_cffi

session = curl_cffi.Session(max_redirects=5)
print(session.get("http://127.0.0.1:18743/t10/start").url)
try:
    session.get("http://127.0.0.1:18743/t10/loop/0")
    print("uncapped")
except curl_cffi.exceptions.TooManyRedirects:
    print("capped")
