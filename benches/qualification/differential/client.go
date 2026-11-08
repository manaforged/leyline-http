package main

import (
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"net/http"
	"os"
	"sync"
	"sync/atomic"
	"time"
)

const workers = 32

func reply(client *http.Client, url string, outcome map[string]any) {
	response, err := client.Get(url)
	if err != nil {
		outcome["error"] = err.Error()
		return
	}
	defer response.Body.Close()
	body, err := io.ReadAll(response.Body)
	if err != nil {
		outcome["error"] = err.Error()
		return
	}
	outcome["status"] = response.StatusCode
	outcome["body"] = hex.EncodeToString(body)
}

func main() {
	port := flag.Int("port", 19000, "server port")
	cases := flag.Int64("cases", 0, "number of cases")
	flag.Parse()
	client := &http.Client{
		Timeout:       3 * time.Second,
		Transport:     &http.Transport{DisableKeepAlives: true, DisableCompression: true},
		CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse },
	}
	var next atomic.Int64
	var lock sync.Mutex
	var group sync.WaitGroup
	encoder := json.NewEncoder(os.Stdout)
	for range workers {
		group.Add(1)
		go func() {
			defer group.Done()
			for {
				current := next.Add(1) - 1
				if current >= *cases {
					return
				}
				outcome := map[string]any{"case": current}
				reply(client, fmt.Sprintf("http://127.0.0.1:%d/c/%d", *port, current), outcome)
				lock.Lock()
				encoder.Encode(outcome)
				lock.Unlock()
			}
		}()
	}
	group.Wait()
}
