import httpx

client = httpx.Client(proxy="http://127.0.0.1:18743")
response = client.get("http://ergonomics.invalid/t09/hello")
print(response.status_code, response.json()["via"])
