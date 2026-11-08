import niquests

session = niquests.Session()
session.post(
    "http://localhost:18743/t03/login", json={"user": "ada", "password": "secret"}
)
print(session.get("http://localhost:18743/t03/me").json()["user"])
