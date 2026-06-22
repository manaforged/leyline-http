// bogdanfinn/tls-client (utls) Chrome-146 head-to-head client.
package main

import (
	"fmt"
	"io"
	"os"
	"strconv"
	"sync"
	"time"

	http "github.com/bogdanfinn/fhttp"
	tls_client "github.com/bogdanfinn/tls-client"
	"github.com/bogdanfinn/tls-client/profiles"
)

func newClient() tls_client.HttpClient {
	opts := []tls_client.HttpClientOption{
		tls_client.WithClientProfile(profiles.Chrome_146),
		tls_client.WithInsecureSkipVerify(),
		tls_client.WithTimeoutSeconds(30),
	}
	c, err := tls_client.NewHttpClient(tls_client.NewNoopLogger(), opts...)
	if err != nil {
		panic(err)
	}
	return c
}

func do(c tls_client.HttpClient, url string) {
	req, err := http.NewRequest(http.MethodGet, url, nil)
	if err != nil {
		panic(err)
	}
	resp, err := c.Do(req)
	if err != nil {
		panic(err)
	}
	io.Copy(io.Discard, resp.Body)
	resp.Body.Close()
}

func main() {
	url := "https://127.0.0.1:8443/"
	warmN, coldN, concN, concC := 2000, 200, 20000, 64
	if len(os.Args) > 1 {
		url = os.Args[1]
	}
	if len(os.Args) > 2 {
		warmN, _ = strconv.Atoi(os.Args[2])
	}
	if len(os.Args) > 3 {
		coldN, _ = strconv.Atoi(os.Args[3])
	}
	if len(os.Args) > 4 {
		concN, _ = strconv.Atoi(os.Args[4])
	}
	if len(os.Args) > 5 {
		concC, _ = strconv.Atoi(os.Args[5])
	}

	if len(os.Args) > 2 && os.Args[2] == "print" {
		req, _ := http.NewRequest(http.MethodGet, url, nil)
		resp, err := newClient().Do(req)
		if err != nil {
			panic(err)
		}
		b, _ := io.ReadAll(resp.Body)
		resp.Body.Close()
		fmt.Println(string(b))
		return
	}

	c := newClient()
	do(c, url)

	start := time.Now()
	for i := 0; i < warmN; i++ {
		do(c, url)
	}
	warmRps := float64(warmN) / time.Since(start).Seconds()

	per := concN / concC
	start = time.Now()
	var wg sync.WaitGroup
	for w := 0; w < concC; w++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for i := 0; i < per; i++ {
				do(c, url)
			}
		}()
	}
	wg.Wait()
	concRps := float64(per*concC) / time.Since(start).Seconds()

	start = time.Now()
	for i := 0; i < coldN; i++ {
		fresh := newClient()
		do(fresh, url)
		fresh.CloseIdleConnections()
	}
	coldRps := float64(coldN) / time.Since(start).Seconds()

	fmt.Printf("RESULT tls-client warm_rps=%.0f conc_rps=%.0f cold_rps=%.0f\n", warmRps, concRps, coldRps)
}
