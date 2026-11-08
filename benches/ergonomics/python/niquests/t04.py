import niquests
from niquests import RetryConfiguration

retry = RetryConfiguration(total=2, backoff_factor=0.1, status_forcelist=[503])
session = niquests.Session(retries=retry)
response = session.get("http://127.0.0.1:18743/t04/flaky")
print(response.status_code, response.json()["attempts"])
