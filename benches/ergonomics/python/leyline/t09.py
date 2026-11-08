import leyline

response = leyline.get(
    "http://ergonomics.invalid/t09/hello", proxy="http://127.0.0.1:18743"
)
print(response.status, response.json()["via"])
