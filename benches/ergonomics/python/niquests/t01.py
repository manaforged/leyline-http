import niquests

user = niquests.get("http://127.0.0.1:18743/t01/user").json()
print(user["id"], user["name"])
