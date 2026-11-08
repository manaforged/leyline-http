from leyline import Retry, Session

response = Session(retry=Retry(2)).get("http://127.0.0.1:18743/t04/flaky")
print(response.status, response.json()["attempts"])
