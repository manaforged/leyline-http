import httpx

client = httpx.Client()
client.post(
    "http://localhost:18743/t03/login", json={"user": "ada", "password": "secret"}
)
print(client.get("http://localhost:18743/t03/me").json()["user"])
