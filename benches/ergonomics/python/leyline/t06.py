from leyline import Retry, Session

session = Session(retry=Retry(2))
print(session.download("http://127.0.0.1:18743/t06/large", "large.bin"))
