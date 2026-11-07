import requests

session = requests.Session()
session.max_redirects = 5
print(session.get("http://127.0.0.1:18743/t10/start").url)
try:
    session.get("http://127.0.0.1:18743/t10/loop/0")
    print("uncapped")
except requests.TooManyRedirects:
    print("capped")
