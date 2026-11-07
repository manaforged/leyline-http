import httpx

body = {"name": "ada", "age": 36}
response = httpx.post("http://127.0.0.1:18743/t02/users", json=body, timeout=5)
print(response.status_code, response.json()["id"])
try:
    httpx.post("http://127.0.0.1:18743/t02/slow", json=body, timeout=0.3)
    print("no timeout")
except httpx.TimeoutException:
    print("timeout")
