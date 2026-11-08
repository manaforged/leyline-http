import requests

proxies = {"http": "http://127.0.0.1:18743"}
response = requests.get("http://ergonomics.invalid/t09/hello", proxies=proxies)
print(response.status_code, response.json()["via"])
