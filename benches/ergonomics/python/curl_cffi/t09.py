import curl_cffi

response = curl_cffi.get(
    "http://ergonomics.invalid/t09/hello", proxy="http://127.0.0.1:18743"
)
print(response.status_code, response.json()["via"])
