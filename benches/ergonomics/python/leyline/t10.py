from leyline import Session

session = Session(redirect=5)
print(session.get("http://127.0.0.1:18743/t10/start").url)
looped = session.get("http://127.0.0.1:18743/t10/loop/0")
print("capped" if looped.status == 302 else "uncapped")
