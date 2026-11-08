import requests
from requests.adapters import HTTPAdapter
from urllib3.util import Retry

session = requests.Session()
retry = Retry(total=2, backoff_factor=0.1, status_forcelist=[503])
session.mount("http://", HTTPAdapter(max_retries=retry))
response = session.get("http://127.0.0.1:18743/t04/flaky")
print(response.status_code, response.json()["attempts"])
