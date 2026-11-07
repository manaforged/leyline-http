import leyline

body = {"name": "ada", "age": 36}
response = leyline.post("http://127.0.0.1:18743/t02/users", json=body, timeout=5)
print(response.status, response.json()["id"])
try:
    leyline.post("http://127.0.0.1:18743/t02/slow", json=body, timeout=0.3)
    print("no timeout")
except leyline.TimeoutError:
    print("timeout")
