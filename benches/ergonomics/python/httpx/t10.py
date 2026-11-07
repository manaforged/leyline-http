import httpx

client = httpx.Client(follow_redirects=True, max_redirects=5)
print(client.get("http://127.0.0.1:18743/t10/start").url)
try:
    client.get("http://127.0.0.1:18743/t10/loop/0")
    print("uncapped")
except httpx.TooManyRedirects:
    print("capped")
