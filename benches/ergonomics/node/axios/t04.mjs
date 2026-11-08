import axios from "axios";
import axiosRetry, { exponentialDelay } from "axios-retry";

axiosRetry(axios, {
  retries: 2,
  retryDelay: exponentialDelay,
  retryCondition: (error) => error.response?.status === 503,
});
const { status, data } = await axios.get("http://127.0.0.1:18743/t04/flaky");
console.log(status, data.attempts);
